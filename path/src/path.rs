//! The default path data structure.

use crate::builder::*;
use crate::math::*;
use crate::geom::Arc;
use crate::{Event, PathEvent, EndpointId, CtrlPointId};

use std::iter::IntoIterator;
use std::mem;
use std::u32;

/// Enumeration corresponding to the [Event](https://docs.rs/lyon_core/*/lyon_core/events/enum.Event.html) enum
/// without the parameters.
///
/// This is used by the [Path](struct.Path.html) data structure to store path events a tad
/// more efficiently.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
enum Verb {
    LineTo,
    QuadraticTo,
    CubicTo,
    Begin,
    Close,
    End,
}

/// A simple path data structure.
///
/// It can be created using a [Builder](struct.Builder.html), and can be iterated over.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
pub struct Path {
    points: Box<[Point]>,
    verbs: Box<[Verb]>,
}

/// A view on a `Path`.
#[derive(Copy, Clone, Debug)]
pub struct PathSlice<'l> {
    points: &'l [Point],
    verbs: &'l [Verb],
}

impl Path {
    /// Creates a [Builder](struct.Builder.html) to create a path.
    pub fn builder() -> Builder { Builder::new() }

    /// Creates an Empty `Path`.
    pub fn new() -> Path {
        Path {
            points: Box::new([]),
            verbs: Box::new([]),
        }
    }

    /// Returns a view on this `Path`.
    pub fn as_slice(&self) -> PathSlice {
        PathSlice {
            points: &self.points[..],
            verbs: &self.verbs[..],
        }
    }

    /// Iterates over the entire `Path`.
    pub fn iter(&self) -> Iter { Iter::new(&self.points[..], &self.verbs[..]) }

    /// Iterates over the endpoint and control point ids of the `Path`.
    pub fn id_iter(&self) -> IdIter { IdIter::new(&self.verbs[..]) }

    pub fn points(&self) -> &[Point] { &self.points[..] }

    pub fn mut_points(&mut self) -> &mut [Point] { &mut self.points[..] }

    /// Concatenate two paths.
    pub fn merge(&self, other: &Self) -> Self {
        let mut verbs = Vec::with_capacity(self.verbs.len() + other.verbs.len());
        let mut points = Vec::with_capacity(self.points.len() + other.points.len());
        verbs.extend_from_slice(&self.verbs);
        verbs.extend_from_slice(&other.verbs);
        points.extend_from_slice(&self.points);
        points.extend_from_slice(&other.points);

        Path {
            verbs: verbs.into_boxed_slice(),
            points: points.into_boxed_slice(),
        }
    }
}

impl std::ops::Index<EndpointId> for Path {
    type Output = Point;
    fn index(&self, id: EndpointId) -> &Point {
        &self.points[id.to_usize()]
    }
}

impl std::ops::Index<CtrlPointId> for Path {
    type Output = Point;
    fn index(&self, id: CtrlPointId) -> &Point {
        &self.points[id.to_usize()]
    }
}

impl<'l> IntoIterator for &'l Path {
    type Item = PathEvent;
    type IntoIter = Iter<'l>;

    fn into_iter(self) -> Iter<'l> { self.iter() }
}

impl<'l> Into<PathSlice<'l>> for &'l Path {
    fn into(self) -> PathSlice<'l> {
        self.as_slice()
    }
}

/// An immutable view over a Path.
impl<'l> PathSlice<'l> {

    pub fn iter<'a>(&'a self) -> Iter<'l> {
        Iter::new(self.points, self.verbs)
    }

    pub fn points(&self) -> &[Point] { self.points }

    pub fn is_empty(&self) -> bool {
        self.verbs.is_empty()
    }
}

impl<'l> IntoIterator for PathSlice<'l> {
    type Item = PathEvent;
    type IntoIter = Iter<'l>;

    fn into_iter(self) -> Iter<'l> { self.iter() }
}

impl<'l, 'a> IntoIterator for &'a PathSlice<'l> {
    type Item = PathEvent;
    type IntoIter = Iter<'l>;

    fn into_iter(self) -> Iter<'l> { self.iter() }
}

/// Builds path object using the FlatPathBuilder interface.
///
/// See the [builder module](builder/index.html) documentation.
pub struct Builder {
    points: Vec<Point>,
    verbs: Vec<Verb>,
    current_position: Point,
    first_position: Point,
    first_vertex: EndpointId,
    first_verb: u32,
    need_moveto: bool,
    last_cmd: Verb,
}

impl Builder {
    pub fn new() -> Self { Builder::with_capacity(0, 0) }

    pub fn with_capacity(points: usize, edges: usize) -> Self {
        Builder {
            points: Vec::with_capacity(points),
            verbs: Vec::with_capacity(edges),
            current_position: Point::new(0.0, 0.0),
            first_position: Point::new(0.0, 0.0),
            first_vertex: EndpointId(0),
            first_verb: 0,
            need_moveto: true,
            last_cmd: Verb::End,
        }
    }

    pub fn with_svg(self) -> SvgPathBuilder<Self> { SvgPathBuilder::new(self) }

    pub fn flattened(self, tolerance: f32) -> FlatteningBuilder<Self> {
        FlatteningBuilder::new(self, tolerance)
    }

    pub fn move_to(&mut self, to: Point) {
        nan_check(to);
        self.end_if_needed();
        self.need_moveto = false;
        self.first_position = to;
        self.first_vertex = EndpointId(self.points.len() as u32);
        self.first_verb = self.verbs.len() as u32;
        self.current_position = to;
        self.points.push(to);
        self.verbs.push(Verb::Begin);
        self.last_cmd = Verb::Begin;
    }

    pub fn line_to(&mut self, to: Point) {
        nan_check(to);
        self.move_to_if_needed();
        self.points.push(to);
        self.verbs.push(Verb::LineTo);
        self.current_position = to;
        self.last_cmd = Verb::LineTo;
    }

    pub fn close(&mut self) {
        // Relative path ops tend to accumulate small floating point imprecisions
        // which results in the last segment ending almost but not quite at the
        // start of the sub-path, causing a new edge to be inserted which often
        // intersects with the first or last edge. This can affect algorithms that
        // Don't handle self-intersecting paths.
        // Deal with this by snapping the last point if it is very close to the
        // start of the sub path.
        if let Some(p) = self.points.last_mut() {
            let d = (*p - self.first_position).abs();
            if d.x + d.y < 0.0001 {
                *p = self.first_position;
            }
        }

        self.verbs.push(Verb::Close);
        self.current_position = self.first_position;
        self.need_moveto = true;
        self.last_cmd = Verb::Close;
    }

    pub fn quadratic_bezier_to(&mut self, ctrl: Point, to: Point) {
        nan_check(ctrl);
        nan_check(to);
        self.move_to_if_needed();
        self.points.push(ctrl);
        self.points.push(to);
        self.verbs.push(Verb::QuadraticTo);
        self.current_position = to;
        self.last_cmd = Verb::QuadraticTo;
    }

    pub fn cubic_bezier_to(&mut self, ctrl1: Point, ctrl2: Point, to: Point) {
        nan_check(ctrl1);
        nan_check(ctrl2);
        nan_check(to);
        self.move_to_if_needed();
        self.points.push(ctrl1);
        self.points.push(ctrl2);
        self.points.push(to);
        self.verbs.push(Verb::CubicTo);
        self.current_position = to;
        self.last_cmd = Verb::CubicTo;
    }

    pub fn arc(
        &mut self,
        center: Point,
        radii: Vector,
        sweep_angle: Angle,
        x_rotation: Angle
    ) {
        nan_check(center);
        nan_check(radii.to_point());
        debug_assert!(!sweep_angle.get().is_nan());
        debug_assert!(!x_rotation.get().is_nan());

        let start_angle = (self.current_position - center).angle_from_x_axis() - x_rotation;
        let arc = Arc { start_angle, center, radii, sweep_angle, x_rotation };
        arc.for_each_quadratic_bezier(&mut|curve| {
            self.quadratic_bezier_to(curve.ctrl, curve.to);
        });
    }

    /// Add a closed polygon.
    pub fn polygon(&mut self, points: &[Point]) {
        self.points.reserve(points.len());
        self.verbs.reserve(points.len() + 1);
        build_polygon(self, points);
    }

    fn move_to_if_needed(&mut self) {
        if self.need_moveto {
            let first = self.first_position;
            self.move_to(first);
        }
    }

    fn end_if_needed(&mut self) {
        if (self.last_cmd as u8) <= (Verb::Begin as u8) {
            self.verbs.push(Verb::End);
        }
    }

    pub fn current_position(&self) -> Point { self.current_position }

    pub fn build(mut self) -> Path {
        self.end_if_needed();
        Path {
            points: self.points.into_boxed_slice(),
            verbs: self.verbs.into_boxed_slice(),
        }
    }
}

pub fn reverse_path(path: PathSlice, builder: &mut dyn PathBuilder) {
    let points = &path.points[..];
    // At each iteration, p points to the first point after the current verb.
    let mut p = points.len();
    let mut need_close = false;

    for v in path.verbs.iter().rev().cloned() {
        match v {
            Verb::Close => {
                need_close = true;
                builder.move_to(points[p - 1]);
            }
            Verb::End => {
                need_close = false;
                builder.move_to(points[p - 1]);
            }
            Verb::Begin => {
                if need_close {
                    need_close = false;
                    builder.close();
                }
            }
            Verb::LineTo => {
                builder.line_to(points[p - 2]);
            }
            Verb::QuadraticTo => {
                builder.quadratic_bezier_to(points[p - 2], points[p - 3]);
            }
            Verb::CubicTo => {
                builder.cubic_bezier_to(points[p - 2], points[p - 3], points[p - 4]);
            }
        }
        p -= n_stored_points(v) as usize;
    }
}

#[test]
fn test_reverse_path() {
    let mut builder = Path::builder();
    builder.move_to(point(0.0, 0.0));
    builder.line_to(point(1.0, 0.0));
    builder.line_to(point(1.0, 1.0));
    builder.line_to(point(0.0, 1.0));

    builder.move_to(point(10.0, 0.0));
    builder.line_to(point(11.0, 0.0));
    builder.line_to(point(11.0, 1.0));
    builder.line_to(point(10.0, 1.0));
    builder.close();

    builder.move_to(point(20.0, 0.0));
    builder.quadratic_bezier_to(point(21.0, 0.0), point(21.0, 1.0));

    let p1 = builder.build();
    let mut builder = Path::builder();
    reverse_path(p1.as_slice(), &mut builder);
    let p2 = builder.build();

    let mut it = p2.iter();

    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(21.0, 1.0) }));
    assert_eq!(it.next(), Some(PathEvent::Quadratic {
        from: point(21.0, 1.0),
        ctrl: point(21.0, 0.0),
        to: point(20.0, 0.0),
    }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(20.0, 0.0), first: point(21.0, 1.0), close: false }));

    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(10.0, 1.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(10.0, 1.0), to: point(11.0, 1.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(11.0, 1.0), to: point(11.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(11.0, 0.0), to: point(10.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(10.0, 0.0), first: point(10.0, 1.0), close: true }));

    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(0.0, 1.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(0.0, 1.0), to: point(1.0, 1.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(1.0, 1.0), to: point(1.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(1.0, 0.0), to: point(0.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(0.0, 0.0), first: point(0.0, 1.0), close: false }));

    assert_eq!(it.next(), None);
}

#[test]
fn test_reverse_path_no_close() {
    let mut builder = Path::builder();
    builder.move_to(point(0.0, 0.0));
    builder.line_to(point(1.0, 0.0));
    builder.line_to(point(1.0, 1.0));

    let p1 = builder.build();

    let mut builder = Path::builder();
    reverse_path(p1.as_slice(), &mut builder);
    let p2 = builder.build();

    let mut it = p2.iter();

    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(1.0, 1.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(1.0, 1.0), to: point(1.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(1.0, 0.0), to: point(0.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(0.0, 0.0), first: point(1.0, 1.0), close: false }));
    assert_eq!(it.next(), None);
}

#[test]
fn test_reverse_empty_path() {
    let p1 = Path::builder().build();
    let mut builder = Path::builder();
    reverse_path(p1.as_slice(), &mut builder);
    let p2 = builder.build();
    assert_eq!(p2.iter().next(), None);
}

#[test]
fn test_reverse_single_moveto() {
    let mut builder = Path::builder();
    builder.move_to(point(0.0, 0.0));
    let p1 = builder.build();
    let mut builder = Path::builder();
    reverse_path(p1.as_slice(), &mut builder);
    let p2 = builder.build();
    let mut it = p2.iter();
    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(0.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(0.0, 0.0), first: point(0.0, 0.0), close: false }));
    assert_eq!(it.next(), None);
}


#[inline]
fn nan_check(p: Point) {
    debug_assert!(p.x.is_finite());
    debug_assert!(p.y.is_finite());
}

impl Build for Builder {
    type PathType = Path;

    fn build(self) -> Path { self.build() }

    fn build_and_reset(&mut self) -> Path {
        self.current_position = Point::new(0.0, 0.0);
        self.first_position = Point::new(0.0, 0.0);

        Path {
            points: mem::replace(&mut self.points, Vec::new()).into_boxed_slice(),
            verbs: mem::replace(&mut self.verbs, Vec::new()).into_boxed_slice(),
        }
    }
}

impl FlatPathBuilder for Builder {
    fn move_to(&mut self, to: Point) {
        self.move_to(to);
    }

    fn line_to(&mut self, to: Point) {
        self.line_to(to);
    }

    fn close(&mut self) {
        self.close();
    }

    fn current_position(&self) -> Point { self.current_position }
}

impl PolygonBuilder for Builder {
    fn polygon(&mut self, points: &[Point]) {
        self.polygon(points);
    }
}

impl PathBuilder for Builder {
    fn quadratic_bezier_to(&mut self, ctrl: Point, to: Point) {
        self.quadratic_bezier_to(ctrl, to);
    }

    fn cubic_bezier_to(&mut self, ctrl1: Point, ctrl2: Point, to: Point) {
        self.cubic_bezier_to(ctrl1, ctrl2, to);
    }

    fn arc(
        &mut self,
        center: Point,
        radii: Vector,
        sweep_angle: Angle,
        x_rotation: Angle
    ) {
        self.arc(center, radii, sweep_angle, x_rotation);
    }
}

/// An iterator for `Path` and `PathSlice`.
#[derive(Clone, Debug)]
pub struct Iter<'l> {
    points: ::std::slice::Iter<'l, Point>,
    verbs: ::std::slice::Iter<'l, Verb>,
    current: Point,
    first: Point,
}

impl<'l> Iter<'l> {
    fn new(points: &'l [Point], verbs: &'l [Verb]) -> Self {
        Iter {
            points: points.iter(),
            verbs: verbs.iter(),
            current: point(0.0, 0.0),
            first: point(0.0, 0.0),
        }
    }
}

impl<'l> Iterator for Iter<'l> {
    type Item = PathEvent;
    fn next(&mut self) -> Option<PathEvent> {
        match self.verbs.next() {
            Some(&Verb::Begin) => {
                self.current = *self.points.next().unwrap();
                self.first = self.current;
                Some(Event::Begin { at: self.current })
            }
            Some(&Verb::LineTo) => {
                let from = self.current;
                self.current = *self.points.next().unwrap();
                Some(Event::Line { from, to: self.current })
            }
            Some(&Verb::QuadraticTo) => {
                let from = self.current;
                let ctrl = *self.points.next().unwrap();
                self.current = *self.points.next().unwrap();
                Some(Event::Quadratic { from, ctrl, to: self.current })
            }
            Some(&Verb::CubicTo) => {
                let from = self.current;
                let ctrl1 = *self.points.next().unwrap();
                let ctrl2 = *self.points.next().unwrap();
                self.current = *self.points.next().unwrap();
                Some(Event::Cubic { from, ctrl1, ctrl2, to: self.current })
            }
            Some(&Verb::Close) => {
                let last = self.current;
                self.current = self.first;
                Some(Event::End { last, first: self.first, close: true, })
            }
            Some(&Verb::End) => {
                let last = self.current;
                self.current = self.first;
                Some(Event::End { last, first: self.first, close: false, })
            }
            None => None,
        }
    }
}

/// An iterator of endpoint and control point ids for `Path` and `PathSlice`.
#[derive(Clone, Debug)]
pub struct IdIter<'l> {
    verbs: ::std::slice::Iter<'l, Verb>,
    current: u32,
    first: u32,
}

impl<'l> IdIter<'l> {
    fn new(verbs: &'l [Verb]) -> Self {
        IdIter {
            verbs: verbs.iter(),
            current: 0,
            first: 0,
        }
    }
}

impl<'l> Iterator for IdIter<'l> {
    type Item = Event<EndpointId, CtrlPointId>; // TODO: IdEvent.
    fn next(&mut self) -> Option<Event<EndpointId, CtrlPointId>> {
        match self.verbs.next() {
            Some(&Verb::Begin) => {
                let at = self.current;
                self.first = at;
                self.current += 1;
                Some(Event::Begin { at: EndpointId(at) })
            }
            Some(&Verb::LineTo) => {
                let from = EndpointId(self.current);
                let to = EndpointId(self.current + 1);
                self.current += 1;
                Some(Event::Line { from, to })
            }
            Some(&Verb::QuadraticTo) => {
                let from = EndpointId(self.current);
                let ctrl = CtrlPointId(self.current + 1);
                let to = EndpointId(self.current + 2);
                self.current += 2;
                Some(Event::Quadratic { from, ctrl, to })
            }
            Some(&Verb::CubicTo) => {
                let from = EndpointId(self.current);
                let ctrl1 = CtrlPointId(self.current + 1);
                let ctrl2 = CtrlPointId(self.current + 2);
                let to = EndpointId(self.current + 3);
                self.current += 3;
                Some(Event::Cubic { from, ctrl1, ctrl2, to })
            }
            Some(&Verb::Close) => {
                let last = EndpointId(self.current);
                let first = EndpointId(self.first);
                self.current = self.first;
                Some(Event::End { last, first, close: true, })
            }
            Some(&Verb::End) => {
                let last = EndpointId(self.current);
                let first = EndpointId(self.first);
                self.current = self.first;
                Some(Event::End { last, first, close: false, })
            }
            None => None,
        }
    }
}

fn n_stored_points(verb: Verb) -> u32 {
    match verb {
        Verb::Begin => 1,
        Verb::LineTo => 1,
        Verb::QuadraticTo => 2,
        Verb::CubicTo => 3,
        Verb::Close => 0,
        Verb::End => 0,
    }
}

#[test]
fn test_path_builder_1() {

    let mut p = Builder::with_capacity(0, 0);
    p.line_to(point(1.0, 0.0));
    p.line_to(point(2.0, 0.0));
    p.line_to(point(3.0, 0.0));
    p.quadratic_bezier_to(point(4.0, 0.0), point(4.0, 1.0));
    p.cubic_bezier_to(point(5.0, 0.0), point(5.0, 1.0), point(5.0, 2.0));
    p.close();

    p.move_to(point(10.0, 0.0));
    p.line_to(point(11.0, 0.0));
    p.line_to(point(12.0, 0.0));
    p.line_to(point(13.0, 0.0));
    p.quadratic_bezier_to(point(14.0, 0.0), point(14.0, 1.0));
    p.cubic_bezier_to(point(15.0, 0.0), point(15.0, 1.0), point(15.0, 2.0));
    p.close();

    p.close();
    p.move_to(point(1.0, 1.0));
    p.move_to(point(2.0, 2.0));
    p.move_to(point(3.0, 3.0));
    p.line_to(point(4.0, 4.0));

    let path = p.build();

    let mut it = path.iter();
    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(0.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(0.0, 0.0), to: point(1.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(1.0, 0.0), to: point(2.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(2.0, 0.0), to: point(3.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Quadratic {
        from: point(3.0, 0.0),
        ctrl: point(4.0, 0.0),
        to: point(4.0, 1.0)
    }));
    assert_eq!(
        it.next(),
        Some(PathEvent::Cubic {
            from: point(4.0, 1.0),
            ctrl1: point(5.0, 0.0),
            ctrl2: point(5.0, 1.0),
            to: point(5.0, 2.0)
        })
    );
    assert_eq!(it.next(), Some(PathEvent::End { last: point(5.0, 2.0), first: point(0.0, 0.0), close: true }));

    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(10.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(10.0, 0.0), to: point(11.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(11.0, 0.0), to: point(12.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(12.0, 0.0), to: point(13.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Quadratic {
        from: point(13.0, 0.0),
        ctrl: point(14.0, 0.0),
        to: point(14.0, 1.0),
    }));
    assert_eq!(
        it.next(),
        Some(PathEvent::Cubic {
            from: point(14.0, 1.0),
            ctrl1: point(15.0, 0.0),
            ctrl2: point(15.0, 1.0),
            to: point(15.0, 2.0),
        })
    );
    assert_eq!(it.next(), Some(PathEvent::End { last: point(15.0, 2.0), first: point(10.0, 0.0), close: true }));

    // Not clear that this is the most useful behavior.
    // Closing when there is no path should probably be dropped.
    assert_eq!(it.next(), Some(PathEvent::End { last: point(10.0, 0.0), first: point(10.0, 0.0), close: true }));

    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(1.0, 1.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(1.0, 1.0), first: point(1.0, 1.0), close: false }));
    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(2.0, 2.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(2.0, 2.0), first: point(2.0, 2.0), close: false }));
    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(3.0, 3.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(3.0, 3.0), to: point(4.0, 4.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(4.0, 4.0), first: point(3.0, 3.0), close: false }));
    assert_eq!(it.next(), None);
    assert_eq!(it.next(), None);
    assert_eq!(it.next(), None);
}

#[test]
fn test_path_builder_empty() {
    let path = Path::builder().build();
    let mut it = path.iter();
    assert_eq!(it.next(), None);
    assert_eq!(it.next(), None);
}

#[test]
fn test_path_builder_empty_move_to() {
    let mut p = Path::builder();
    p.move_to(point(1.0, 2.0));
    p.move_to(point(3.0, 4.0));
    p.move_to(point(5.0, 6.0));

    let path = p.build();
    let mut it = path.iter();
    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(1.0, 2.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(1.0, 2.0), first: point(1.0, 2.0), close: false, }));
    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(3.0, 4.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(3.0, 4.0), first: point(3.0, 4.0), close: false, }));
    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(5.0, 6.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(5.0, 6.0), first: point(5.0, 6.0), close: false, }));
    assert_eq!(it.next(), None);
    assert_eq!(it.next(), None);
}

#[test]
fn test_path_builder_line_to_after_close() {
    let mut p = Path::builder();
    p.line_to(point(1.0, 0.0));
    p.close();
    p.line_to(point(2.0, 0.0));

    let path = p.build();
    let mut it = path.iter();
    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(0.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(0.0, 0.0), to: point(1.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(1.0, 0.0), first: point(0.0, 0.0), close: true }));
    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(0.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(0.0, 0.0), to: point(2.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(2.0, 0.0), first: point(0.0, 0.0), close: false }));
    assert_eq!(it.next(), None);
}

/// Builder for flattened paths
pub type FlattenedPathBuilder = SvgPathBuilder<FlatteningBuilder<Builder>>;
/// FlattenedPathBuilder constructor.
pub fn flattened_path_builder(tolerance: f32) -> FlattenedPathBuilder {
    SvgPathBuilder::new(FlatteningBuilder::new(Path::builder(), tolerance))
}

#[test]
fn test_merge_paths() {
    let mut builder = Path::builder();
    builder.move_to(point(0.0, 0.0));
    builder.line_to(point(5.0, 0.0));
    builder.line_to(point(5.0, 5.0));
    builder.close();

    let path1 = builder.build();

    let mut builder = Path::builder();
    builder.move_to(point(1.0, 1.0));
    builder.line_to(point(4.0, 0.0));
    builder.line_to(point(4.0, 4.0));
    builder.close();

    let path2 = builder.build();

    let path = path1.merge(&path2);

    let mut it = path.iter();
    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(0.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(0.0, 0.0), to: point(5.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(5.0, 0.0), to: point(5.0, 5.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(5.0, 5.0), first: point(0.0, 0.0), close: true }));
    assert_eq!(it.next(), Some(PathEvent::Begin { at: point(1.0, 1.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(1.0, 1.0), to: point(4.0, 0.0) }));
    assert_eq!(it.next(), Some(PathEvent::Line { from: point(4.0, 0.0), to: point(4.0, 4.0) }));
    assert_eq!(it.next(), Some(PathEvent::End { last: point(4.0, 4.0), first: point(1.0, 1.0), close: true }));
    assert_eq!(it.next(), None);
}
