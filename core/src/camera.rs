//! The camera that the gesture recognizer moves: it reads the map's latest
//! snapshot, and sends each change to the map as a command, as a host of
//! maplibre-native-ffi's C API does.

use mln_gestures::GestureCamera;
use mln_map::{CameraDelta, CameraOptions, CameraUpdate, MapProjection, TransformState};
use mln_util::LatLng;
use mln_util::geo::{ScreenCoordinate, WrapMode};

pub(super) struct SnapshotCamera<'a> {
    pub(super) map: &'a mln_runtime::Map,
    pub(super) state: &'a TransformState,
    pub(super) projection: &'a MapProjection,
}

impl GestureCamera for SnapshotCamera<'_> {
    fn viewport(&self) -> ScreenCoordinate {
        let size = self.state.get_size();
        ScreenCoordinate::new(f64::from(size.width), f64::from(size.height))
    }

    fn camera(&self) -> CameraOptions {
        self.state.get_camera_options(None)
    }

    /// As mln_gestures finds it for a map.
    fn zoom_range(&self) -> (f64, f64) {
        let latitude = self.state.get_lat_lng(WrapMode::Unwrapped).latitude();
        let min = self.state.get_min_zoom_at_latitude(latitude);
        let min = self.state.get_min_zoom_for_screen_bounds().map_or(min, |bounds| min.max(bounds));
        let max = self.state.get_max_zoom();
        (min.min(max), max)
    }

    fn lat_lng_for_pixel(&self, pixel: ScreenCoordinate) -> Option<LatLng> {
        self.projection.lat_lng_for_pixel(&pixel, WrapMode::Unwrapped).ok()
    }

    fn apply_camera_delta(&mut self, delta: &CameraDelta) {
        let _ = self.map.apply_camera_delta(delta.clone());
    }

    fn update_camera(&mut self, update: &CameraUpdate) {
        let _ = self.map.update_camera(update.clone());
    }

    fn cancel_transitions_with_id(&mut self, transition_id: u64) {
        let _ = self.map.cancel_camera_transitions_with_id(transition_id);
    }
}
