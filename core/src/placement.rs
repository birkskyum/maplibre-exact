//! Where a marker puts a view laid out over the map, as GL JS's `Marker`
//! places its element: the anchor at the marker's point plus the offset, or
//! where the layout put it, turned by the rotation, less the bearing when it
//! turns with the map, and squashed as the map pitches when it lies on the
//! map. A host moves the view there, or draws its pixels there.

use mln_map::MapProjection;
use mln_util::geo::LatLng;

use crate::model::Marker;

/// Where a view goes: `homography` maps the view's points to the map's,
/// row major. A hidden view lies off the map or behind the globe, so the host
/// neither shows it nor hit-tests it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub hidden: bool,
    pub homography: [f32; 9],
}

/// The map that markers are placed on, as one frame drew it.
pub struct View<'a> {
    pub projection: &'a MapProjection,
    /// The map's bearing and pitch, in degrees.
    pub bearing: f64,
    pub pitch: f64,
    /// The map in points.
    pub size: [f32; 2],
    /// Pixels a point.
    pub scale: f32,
}

/// Where `marker` puts a view laid out at `frame` (x, y, width and height in
/// points). An upright view sits on whole pixels, so that its pixels stay
/// sharp. `None` leaves the view to the layout.
#[must_use]
pub fn place(view: &View<'_>, marker: &Marker, frame: [f32; 4]) -> Option<Placement> {
    let projection = view.projection;
    let [width, height] = [f64::from(frame[2]), f64::from(frame[3])];
    let [ax, ay] = marker.anchor.fraction();
    let [ax, ay] = [ax * width, ay * height];
    let angle = (marker.rotation - if marker.turns_with_map() { view.bearing } else { 0.0 }).to_radians();
    let squash = if marker.lies_on_map() { view.pitch.to_radians().cos() } else { 1.0 };
    let upright = angle.rem_euclid(std::f64::consts::TAU) == 0.0 && squash == 1.0;
    let point = match marker.lng_lat {
        Some([lng, lat]) => LatLng::new(lat, lng)
            .ok()
            .filter(|at| !projection.is_location_occluded(at))
            .map(|at| projection.pixel_for_lat_lng(&at))
            .filter(|point| point.x.is_finite() && point.y.is_finite())
            .map(|point| [point.x, point.y]),
        None if upright && marker.offset == [0.0, 0.0] => return None,
        None => Some([f64::from(frame[0]) + ax, f64::from(frame[1]) + ay]),
    };
    let Some([x, y]) = point.map(|[x, y]| [x + marker.offset[0], y + marker.offset[1]]) else {
        return Some(Placement { hidden: true, ..at(1.0, 0.0, 0.0, 1.0, 0.0, 0.0) });
    };
    // The child turned about its anchor, squashed, then moved to the point.
    let (sin, cos) = angle.sin_cos();
    let [a, b, c, d] = [cos, -sin, squash * sin, squash * cos];
    let [mut tx, mut ty] = [x - (a * ax + b * ay), y - (c * ax + d * ay)];
    if upright {
        let snap = |n: f64| (n * f64::from(view.scale)).round() / f64::from(view.scale);
        [tx, ty] = [snap(tx), snap(ty)];
    }
    let mut placement = at(a, b, c, d, tx, ty);
    let corners = [[0.0, 0.0], [width, 0.0], [0.0, height], [width, height]]
        .map(|[u, v]| [a * u + b * v + tx, c * u + d * v + ty]);
    let [x0, y0, x1, y1] = corners.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |[x0, y0, x1, y1], [x, y]| {
        [x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y)]
    });
    placement.hidden = x1 < 0.0 || y1 < 0.0 || x0 > f64::from(view.size[0]) || y0 > f64::from(view.size[1]);
    Some(placement)
}

/// The placement of the affine map `[a b tx; c d ty]`.
fn at(a: f64, b: f64, c: f64, d: f64, tx: f64, ty: f64) -> Placement {
    Placement { hidden: false, homography: [a, b, tx, c, d, ty, 0.0, 0.0, 1.0].map(|n| n as f32) }
}

#[cfg(test)]
mod tests {
    use mln_map::{CameraOptions, Map, MapOptions};
    use mln_util::size::Size;

    use super::*;
    use crate::model::{Alignment, Anchor};

    /// A 400×300 map looking straight down at 0, 0 at zoom 2, turned and
    /// pitched as given.
    fn map(bearing: f64, pitch: f64) -> MapProjection {
        let options = MapOptions { size: Size::new(400, 300), pixel_ratio: 1.0, ..MapOptions::default() };
        let mut map = Map::new(options).unwrap();
        map.jump_to(&CameraOptions {
            center: LatLng::new(0.0, 0.0).ok(),
            zoom: Some(2.0),
            bearing: Some(bearing),
            pitch: Some(pitch),
            ..CameraOptions::default()
        });
        MapProjection::new(&map)
    }

    fn view(projection: &MapProjection, bearing: f64, pitch: f64) -> View<'_> {
        View { projection, bearing, pitch, size: [400.0, 300.0], scale: 2.0 }
    }

    fn marker(lng_lat: Option<[f64; 2]>) -> Marker {
        Marker {
            id: "m".into(),
            lng_lat,
            anchor: Anchor::Center,
            offset: [0.0, 0.0],
            rotation: 0.0,
            rotation_alignment: Alignment::Auto,
            pitch_alignment: Alignment::Auto,
        }
    }

    fn near(a: [f32; 9], b: [f32; 9]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
    }

    #[test]
    fn a_marker_puts_its_anchor_at_its_place_plus_the_offset() {
        let projection = map(0.0, 0.0);
        let view = view(&projection, 0.0, 0.0);
        let callout = Marker { anchor: Anchor::Bottom, offset: [0.0, -4.0], ..marker(Some([0.0, 0.0])) };
        let placement = place(&view, &callout, [0.0, 0.0, 80.0, 40.0]).unwrap();
        assert!(!placement.hidden);
        assert!(near(placement.homography, [1.0, 0.0, 160.0, 0.0, 1.0, 106.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn a_marker_off_the_canvas_is_hidden() {
        let projection = map(0.0, 0.0);
        let view = view(&projection, 0.0, 0.0);
        let placement = place(&view, &marker(Some([120.0, 0.0])), [0.0, 0.0, 80.0, 40.0]).unwrap();
        assert!(placement.hidden);
    }

    #[test]
    fn an_upright_marker_without_a_place_is_left_to_the_layout() {
        let projection = map(0.0, 0.0);
        let view = view(&projection, 0.0, 0.0);
        let compass = Marker { rotation_alignment: Alignment::Map, ..marker(None) };
        assert_eq!(place(&view, &compass, [354.0, 10.0, 36.0, 36.0]), None);
    }

    #[test]
    fn a_compass_turns_against_the_bearing_about_its_center_and_lies_on_the_map() {
        let projection = map(90.0, 60.0);
        let view = view(&projection, 90.0, 60.0);
        let compass = Marker { rotation_alignment: Alignment::Map, ..marker(None) };
        let placement = place(&view, &compass, [354.0, 10.0, 36.0, 36.0]).unwrap();
        // Turned a quarter counterclockwise and squashed to half its height
        // (cos 60°), its center where the layout put it.
        let h = placement.homography;
        assert!(near(h, [0.0, 1.0, h[2], -0.5, 0.0, h[5], 0.0, 0.0, 1.0]), "{h:?}");
        let center = [h[0] * 18.0 + h[1] * 18.0 + h[2], h[3] * 18.0 + h[4] * 18.0 + h[5]];
        assert!((center[0] - 372.0).abs() < 1e-3 && (center[1] - 28.0).abs() < 1e-3, "{center:?}");
    }
}
