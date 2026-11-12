//! What a maplibre-exact map takes and sends, whichever engine draws it.
//!
//! A map is a canvas surface with named arguments ([`ARGUMENTS`]). A camera
//! argument moves the map only when its value changes, so the person can pan
//! freely and the app moves the map again by changing the value. Commands
//! ([`Command`]) are JSON texts the app posts to the surface with
//! `postMessage(text, "maplibre")`, and events ([`Event`]) are JSON texts the
//! surface posts to the canvas's `message=` action. The names follow MapLibre
//! GL JS: `[longitude, latitude]` pairs, `jumpTo`, `easeTo`, `flyTo`,
//! `moveend`.

use exact_gpu::Value;
use serde::{Deserialize, Serialize};

/// The surface's arguments in bind order, with their defaults.
pub const ARGUMENTS: [&str; 6] = ["style", "longitude", "latitude", "zoom", "bearing", "pitch"];

/// How many arguments the surface binds.
pub const ARITY: usize = ARGUMENTS.len();

/// The arguments with their defaults, as `Surface::arguments` declares them.
/// The style has none: an empty style is refused.
#[must_use]
pub fn argument_defaults() -> Vec<(&'static str, Value)> {
    vec![
        ("style", Value::from("")),
        ("longitude", Value::Number(0.0)),
        ("latitude", Value::Number(0.0)),
        ("zoom", Value::Number(0.0)),
        ("bearing", Value::Number(0.0)),
        ("pitch", Value::Number(0.0)),
    ]
}

/// The bound arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct Inputs {
    /// A style URL, or a style document as JSON.
    pub style: String,
    pub camera: Camera,
}

impl Inputs {
    /// The arguments in [`ARGUMENTS`] order.
    ///
    /// # Errors
    ///
    /// The message names the argument that is missing or of the wrong type.
    pub fn from_values(values: &[Value]) -> Result<Inputs, String> {
        let [style, longitude, latitude, zoom, bearing, pitch] = values else {
            return Err(format!("maplibre: expected {ARITY} arguments, got {}", values.len()));
        };
        let number = |value: &Value, name: &str| {
            value
                .as_number()
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("maplibre: {name} must be a finite number"))
        };
        let style = style.as_str().ok_or("maplibre: style must be text")?.trim().to_owned();
        if style.is_empty() {
            return Err("maplibre: style is empty; name a style URL or pass a style document".into());
        }
        Ok(Inputs {
            style,
            camera: Camera {
                center: [number(longitude, "longitude")?, number(latitude, "latitude")?],
                zoom: number(zoom, "zoom")?,
                bearing: number(bearing, "bearing")?,
                pitch: number(pitch, "pitch")?,
            },
        })
    }

    /// Whether the style is a document rather than a URL.
    #[must_use]
    pub fn style_is_document(&self) -> bool {
        self.style.starts_with('{')
    }
}

/// Where the map looks.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Camera {
    /// `[longitude, latitude]` in degrees.
    pub center: [f64; 2],
    pub zoom: f64,
    /// Degrees clockwise from north.
    pub bearing: f64,
    /// Degrees from looking straight down.
    pub pitch: f64,
}

/// The parts of a camera a command sets; the rest stay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
pub struct CameraChange {
    pub center: Option<[f64; 2]>,
    pub zoom: Option<f64>,
    pub bearing: Option<f64>,
    pub pitch: Option<f64>,
}

/// A camera change that animates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
pub struct Animation {
    #[serde(flatten)]
    pub camera: CameraChange,
    /// Milliseconds; the engine's own duration when absent.
    pub duration: Option<f64>,
}

/// A command the app posts, as `{"command": "flyTo", "center": [lng, lat], …}`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "command", rename_all = "camelCase")]
pub enum Command {
    JumpTo(CameraChange),
    EaseTo(Animation),
    FlyTo(Animation),
    SetStyle { style: String },
}

impl Command {
    /// The command a posted text names.
    ///
    /// # Errors
    ///
    /// The message says why the text is no command.
    pub fn parse(text: &str) -> Result<Command, String> {
        serde_json::from_str(text).map_err(|e| format!("maplibre: {e} in the command {text}"))
    }
}

/// An event the surface posts to the app, as `{"event": "moveend", …}`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Event {
    /// The style loaded and the map drew it.
    Load,
    /// The camera came to rest, whoever moved it.
    Moveend(Camera),
    /// A press and release that didn't move the map.
    Click {
        #[serde(rename = "lngLat")]
        lng_lat: [f64; 2],
        /// Points from the canvas's top-left corner.
        point: [f64; 2],
    },
    Error { message: String },
}

impl Event {
    /// The text the app receives.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// The surface's record, which the app reads as `exactSurface("maplibre")`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Record {
    #[serde(flatten)]
    pub camera: Camera,
    /// Whether the style and every tile in view have loaded.
    pub loaded: bool,
}

impl Record {
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fly_to_takes_its_camera_and_duration() {
        let command = Command::parse(r#"{"command":"flyTo","center":[8.54,47.37],"zoom":12,"duration":2000}"#);
        assert_eq!(
            command,
            Ok(Command::FlyTo(Animation {
                camera: CameraChange { center: Some([8.54, 47.37]), zoom: Some(12.0), ..CameraChange::default() },
                duration: Some(2000.0),
            }))
        );
    }

    #[test]
    fn an_unknown_command_is_refused() {
        assert!(Command::parse(r#"{"command":"spin"}"#).is_err());
    }

    #[test]
    fn a_moveend_carries_the_camera_beside_its_name() {
        let camera = Camera { center: [1.0, 2.0], zoom: 3.0, bearing: 0.0, pitch: 0.0 };
        assert_eq!(
            Event::Moveend(camera).to_json(),
            r#"{"event":"moveend","center":[1.0,2.0],"zoom":3.0,"bearing":0.0,"pitch":0.0}"#
        );
    }

    #[test]
    fn an_empty_style_is_refused() {
        let mut values: Vec<Value> = argument_defaults().into_iter().map(|(_, v)| v).collect();
        assert!(Inputs::from_values(&values).is_err());
        values[0] = Value::from("https://example.com/style.json");
        assert!(Inputs::from_values(&values).is_ok());
    }
}
