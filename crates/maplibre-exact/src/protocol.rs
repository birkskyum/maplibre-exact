//! What a maplibre-exact map takes and sends, whichever engine draws it.
//!
//! A map is a canvas surface with named arguments ([`ARGUMENTS`]). The style
//! is the base the map starts from, and `content` adds the app's own sources
//! and layers over it ([`Content`]); when the content changes, the map changes
//! to match it, and a new style keeps it. A camera argument moves the map only
//! when its value changes, so the person can pan freely and the app moves the
//! map again by changing the value.
//!
//! The app sends commands ([`Command`]) as JSON texts with
//! `postMessage(text, "maplibre")`. The map posts the name of each event
//! ([`Event`]) to the canvas's `message=` action, and keeps its camera, the
//! last click and the last error in its record ([`Record`]), which the app
//! reads with `exactSurface("maplibre")` as a shape of its own. The names
//! follow MapLibre GL JS and the MapLibre Style Specification.

use exact_gpu::Value;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

/// The surface's arguments in bind order.
pub const ARGUMENTS: [&str; 7] = ["style", "longitude", "latitude", "zoom", "bearing", "pitch", "content"];

/// How many arguments the surface binds.
pub const ARITY: usize = ARGUMENTS.len();

/// The arguments with their defaults, as `Surface::arguments` declares them.
/// The style has none, and an empty style is refused.
#[must_use]
pub fn argument_defaults() -> Vec<(&'static str, Value)> {
    vec![
        ("style", Value::from("")),
        ("longitude", Value::Number(0.0)),
        ("latitude", Value::Number(0.0)),
        ("zoom", Value::Number(0.0)),
        ("bearing", Value::Number(0.0)),
        ("pitch", Value::Number(0.0)),
        ("content", Value::from("")),
    ]
}

/// The bound arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct Inputs {
    /// A style URL, or a style document as JSON.
    pub style: String,
    pub camera: Camera,
    pub content: Content,
}

impl Inputs {
    /// The arguments in [`ARGUMENTS`] order.
    ///
    /// # Errors
    ///
    /// The message names the argument that is missing, of the wrong type or
    /// malformed.
    pub fn from_values(values: &[Value]) -> Result<Inputs, String> {
        let [style, longitude, latitude, zoom, bearing, pitch, content] = values else {
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
            content: Content::parse(content.as_str().ok_or("maplibre: content must be text")?)?,
        })
    }

    /// Whether the style is a document rather than a URL.
    #[must_use]
    pub fn style_is_document(&self) -> bool {
        self.style.starts_with('{')
    }
}

/// The app's sources and layers, drawn over the base style, as
/// `{"sources": {"id": source, …}, "layers": [layer, …]}` in the style
/// specification's terms. A layer draws above the layers before it and above
/// the base style, unless it names a layer to draw under with `beforeId`, as
/// GL JS's `addLayer` takes it. A layer may use a source of the base style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Content {
    /// Sources by id.
    pub sources: Vec<(String, Json)>,
    /// Layers in drawing order, the last on top.
    pub layers: Vec<Layer>,
    /// How many points around a click still hit a feature, which makes small
    /// circles and thin lines easier to tap.
    pub hit_padding: f64,
}

/// One of the app's layers.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub id: String,
    /// The layer it draws under, if not above all before it.
    pub before: Option<String>,
    /// The layer as the style specification writes it, without `beforeId`.
    pub json: Json,
}

impl Content {
    /// The content in `text`; empty text is no content.
    ///
    /// # Errors
    ///
    /// The message says what in the text is malformed.
    pub fn parse(text: &str) -> Result<Content, String> {
        if text.trim().is_empty() {
            return Ok(Content::default());
        }
        let value: Json = serde_json::from_str(text).map_err(|e| format!("maplibre: content: {e}"))?;
        let Json::Object(mut object) = value else {
            return Err("maplibre: content must be a JSON object".into());
        };
        let mut content = Content::default();
        match object.remove("sources") {
            None => {}
            Some(Json::Object(sources)) => {
                for (id, source) in sources {
                    if !source.is_object() {
                        return Err(format!("maplibre: content source {id} must be an object"));
                    }
                    content.sources.push((id, source));
                }
            }
            Some(_) => return Err("maplibre: content's sources must be an object of sources by id".into()),
        }
        match object.remove("layers") {
            None => {}
            Some(Json::Array(layers)) => {
                for (index, layer) in layers.into_iter().enumerate() {
                    let Json::Object(mut layer) = layer else {
                        return Err(format!("maplibre: content layer {index} must be an object"));
                    };
                    let id = match layer.get("id") {
                        Some(Json::String(id)) if !id.is_empty() => id.clone(),
                        _ => return Err(format!("maplibre: content layer {index} has no id")),
                    };
                    if content.layers.iter().any(|l| l.id == id) {
                        return Err(format!("maplibre: content has two layers with the id {id}"));
                    }
                    let before = match layer.remove("beforeId") {
                        None | Some(Json::Null) => None,
                        Some(Json::String(before)) => Some(before),
                        Some(_) => return Err(format!("maplibre: layer {id}'s beforeId must be a layer id")),
                    };
                    content.layers.push(Layer { id, before, json: Json::Object(layer) });
                }
            }
            Some(_) => return Err("maplibre: content's layers must be an array".into()),
        }
        match object.remove("hitPadding") {
            None => {}
            Some(Json::Number(n)) if n.as_f64().is_some_and(|n| n >= 0.0) => {
                content.hit_padding = n.as_f64().unwrap_or_default();
            }
            Some(_) => return Err("maplibre: content's hitPadding must be a number of points, 0 or more".into()),
        }
        Ok(content)
    }

    /// The source with `id`, if the content has one.
    #[must_use]
    pub fn source(&self, id: &str) -> Option<&Json> {
        self.sources.iter().find(|(source, _)| source == id).map(|(_, source)| source)
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

/// What the map posts to the canvas's `message=` action, as its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The style loaded and the map drew it.
    Load,
    /// The camera came to rest, whoever moved it.
    Moveend,
    /// A press and release that didn't move the map, which the record's
    /// `click` describes.
    Click,
    /// Something failed, which the record's `error` says.
    Error,
}

impl Event {
    /// The text the app receives.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Event::Load => "load",
            Event::Moveend => "moveend",
            Event::Click => "click",
            Event::Error => "error",
        }
    }
}

/// The map's record, which the app reads as `exactSurface("maplibre")`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Record {
    /// The camera where it last came to rest.
    #[serde(flatten)]
    pub camera: Camera,
    /// Whether the style and every tile in view have loaded.
    pub loaded: bool,
    /// The last click, `null` before the first.
    pub click: Option<Click>,
    /// The last error, `null` while there has been none.
    pub error: Option<String>,
}

impl Record {
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// A click on the map.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Click {
    #[serde(rename = "lngLat")]
    pub lng_lat: [f64; 2],
    /// Points from the canvas's top-left corner.
    pub point: [f64; 2],
    /// The features of the content's layers at the click, topmost first.
    pub features: Vec<Feature>,
}

/// A feature a layer drew, as GL JS's `queryRenderedFeatures` returns it,
/// without its geometry.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Feature {
    /// The feature's id, `null` when it has none.
    pub id: Json,
    pub layer: String,
    pub source: String,
    #[serde(rename = "sourceLayer")]
    pub source_layer: String,
    pub properties: Map<String, Json>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(content: &str) -> Result<Inputs, String> {
        let mut values: Vec<Value> = argument_defaults().into_iter().map(|(_, v)| v).collect();
        values[0] = Value::from("https://example.com/style.json");
        values[6] = Value::from(content);
        Inputs::from_values(&values)
    }

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
    fn an_empty_style_is_refused() {
        let values: Vec<Value> = argument_defaults().into_iter().map(|(_, v)| v).collect();
        assert!(Inputs::from_values(&values).is_err());
        assert!(inputs("").is_ok());
    }

    #[test]
    fn content_keeps_its_layers_in_order_and_takes_before_ids_out() {
        let content = inputs(
            r#"{"sources":{"trail":{"type":"geojson","data":{"type":"FeatureCollection","features":[]}}},
                "layers":[{"id":"casing","type":"line","source":"trail","beforeId":"labels"},
                          {"id":"line","type":"line","source":"trail"}],
                "hitPadding":6}"#,
        )
        .map(|inputs| inputs.content);
        let Ok(content) = content else { panic!("{content:?}") };
        assert_eq!(content.sources.len(), 1);
        assert_eq!(content.layers.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["casing", "line"]);
        assert_eq!(content.layers[0].before.as_deref(), Some("labels"));
        assert!(content.layers[0].json.get("beforeId").is_none());
        assert_eq!(content.hit_padding, 6.0);
    }

    #[test]
    fn content_with_two_layers_of_one_id_is_refused() {
        assert!(inputs(r#"{"layers":[{"id":"a","type":"background"},{"id":"a","type":"background"}]}"#).is_err());
    }

    #[test]
    fn the_record_has_null_for_no_click_and_no_error() {
        let record = Record {
            camera: Camera { center: [1.0, 2.0], zoom: 3.0, bearing: 0.0, pitch: 0.0 },
            loaded: true,
            click: None,
            error: None,
        };
        assert_eq!(
            record.to_json(),
            r#"{"center":[1.0,2.0],"zoom":3.0,"bearing":0.0,"pitch":0.0,"loaded":true,"click":null,"error":null}"#
        );
    }
}
