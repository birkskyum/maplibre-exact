//! A map component's state, whichever platform shows it: the map its props
//! declare, the engine once the view has a size, and the input and frames
//! the view hands it.

use std::collections::HashMap;
use std::time::Instant;

use maplibre_rust_ui::gestures::{Button, Input, Modifiers, PointerKind};
use maplibre_rust_ui::model::{Camera, Command, Content, Event, Inputs, Options, Record};
use maplibre_rust_ui::runtime::{Driver, RenderTarget};
use maplibre_rust_ui::util::geo::ScreenCoordinate;
use maplibre_rust_ui::util::image::PremultipliedImage;
use maplibre_rust_ui::{Engine, Phase, Pointer, Viewport};
use serde_json::Value as Json;

/// The events a map posts when its props name none.
const ALL_EVENTS: [Event; 5] = [Event::Load, Event::Moveend, Event::Click, Event::Contextmenu, Event::Error];

/// Where the camera is before the props set one: the whole world.
const WORLD: Camera = Camera { center: [0.0, 0.0], zoom: 0.0, bearing: 0.0, pitch: 0.0 };

/// What a map posts to its app: an event's name, with the map's record.
pub(crate) struct Post {
    pub name: &'static str,
    pub data: Json,
}

/// A pointer as the view reports it.
pub(crate) struct Press {
    pub id: u32,
    pub phase: Phase,
    pub x: f64,
    pub y: f64,
    pub button: Button,
    pub touch: bool,
    pub modifiers: Modifiers,
}

pub(crate) struct MapState {
    inputs: Inputs,
    engine: Option<Engine>,
    /// Commands sent before the map was made.
    pending: Vec<Command>,
    /// The content's images, decoded, by the paths the content names them by.
    images: HashMap<String, PremultipliedImage>,
    /// The view's size, once it has one.
    viewport: Option<Viewport>,
    /// Failures of the props and commands, until the app hears of them.
    failures: Vec<String>,
    /// Why the map couldn't be made, once it couldn't.
    refused: Option<String>,
    epoch: Instant,
}

impl MapState {
    pub fn new() -> MapState {
        MapState {
            inputs: Inputs {
                style: String::new(),
                camera: WORLD,
                content: Content::default(),
                options: Options::default(),
                events: ALL_EVENTS.to_vec(),
            },
            engine: None,
            pending: Vec::new(),
            images: HashMap::new(),
            viewport: None,
            failures: Vec::new(),
            refused: None,
            epoch: Instant::now(),
        }
    }

    /// Prop `name` changed to `value`, or went with `None`.
    pub fn set_prop(&mut self, name: &str, value: Option<&Json>) {
        let changed = match name {
            "style" => {
                self.inputs.style = match value {
                    Some(Json::String(url)) => url.clone(),
                    Some(document) => document.to_string(),
                    None => String::new(),
                };
                Ok(())
            }
            "camera" => camera(value).map(|camera| self.inputs.camera = camera),
            "content" => Content::parse(&text(value)).map(|content| self.inputs.content = content),
            "options" => Options::parse(&text(value)).map(|options| self.inputs.options = options),
            "events" => events(value).map(|events| self.inputs.events = events),
            _ => Ok(()),
        };
        if let Err(message) = changed {
            self.failures.push(message);
        }
    }

    /// The app called `command(name, args)` on the map's ref, with GL JS's
    /// camera options in `args`.
    pub fn command(&mut self, name: &str, args: &Json) {
        let mut json = match args {
            Json::Object(fields) => Json::Object(fields.clone()),
            Json::Null => Json::Object(serde_json::Map::new()),
            _ => {
                self.failures.push(format!("maplibre: {name}: its options are an object"));
                return;
            }
        };
        json["command"] = Json::from(name);
        match Command::parse(&json.to_string()) {
            Ok(command) => {
                let at_ms = self.now_ms();
                match self.engine.as_mut() {
                    Some(engine) => engine.command(command, at_ms),
                    None => self.pending.push(command),
                }
            }
            Err(message) => self.failures.push(message),
        }
    }

    /// The view is `viewport` now, or has no area with `None`.
    pub fn resize(&mut self, viewport: Option<Viewport>) {
        self.viewport = viewport;
        if let (Some(engine), Some(viewport)) = (self.engine.as_mut(), viewport)
            && let Err(message) = engine.resize(viewport)
        {
            self.failures.push(message);
        }
    }

    /// A pointer on the view, which the map follows for clicks and long
    /// presses, and its gestures for moving the map while it is interactive.
    pub fn pointer(&mut self, press: &Press) {
        let at_ms = self.now_ms();
        let Some(engine) = self.engine.as_mut() else { return };
        let Press { id, phase, x, y, button, touch, modifiers } = *press;
        engine.pointer(&Pointer { id, phase, x, y, touch, secondary: button == Button::Secondary, at_ms });
        if !engine.interactive() {
            return;
        }
        let position = ScreenCoordinate::new(x, y);
        let input = match phase {
            Phase::Down => Input::PointerDown {
                id,
                kind: if touch { PointerKind::Touch } else { PointerKind::Mouse },
                button,
                position,
                modifiers,
            },
            Phase::Move => Input::PointerMove { id, position },
            Phase::Up => Input::PointerUp { id, position },
            Phase::Cancel => Input::PointerCancel { id },
        };
        engine.gesture(input, at_ms);
    }

    /// Input other than pointers, such as a wheel or a trackpad's pinch, for
    /// the gestures while the map is interactive.
    pub fn gesture(&mut self, input: Input) {
        let at_ms = self.now_ms();
        if let Some(engine) = self.engine.as_mut()
            && engine.interactive()
        {
            engine.gesture(input, at_ms);
        }
    }

    /// Runs a frame: makes the map once the view has a size and the props a
    /// style, rendering into the target `target` makes for its viewport,
    /// then takes the props and runs the gestures and the map. Returns
    /// whether the map wants another frame, and what to post to the app.
    pub fn frame(
        &mut self,
        target: impl FnOnce(Viewport) -> Result<(RenderTarget, Driver), String>,
    ) -> (bool, Vec<Post>) {
        let now_ms = self.now_ms();
        if self.engine.is_none()
            && self.refused.is_none()
            && !self.inputs.style.is_empty()
            && let Some(viewport) = self.viewport
        {
            let made = target(viewport)
                .and_then(|(target, driver)| Engine::new(viewport, now_ms, self.inputs.clone(), target, driver));
            match made {
                Ok(mut engine) => {
                    for command in self.pending.drain(..) {
                        engine.command(command, now_ms);
                    }
                    self.engine = Some(engine);
                }
                Err(message) => {
                    self.refused = Some(message.clone());
                    self.failures.push(message);
                }
            }
        }
        let mut events = Vec::new();
        let mut again = false;
        if let Some(engine) = self.engine.as_mut() {
            engine.apply_inputs(&self.inputs);
            again = engine.frame(now_ms, &self.images, &mut events);
            if let Some(refusal) = engine.take_refusal() {
                self.failures.push(refusal);
            }
        }
        let mut record = self.engine.as_ref().map(Engine::record).unwrap_or_else(|| Record {
            camera: self.inputs.camera,
            loaded: false,
            click: None,
            contextmenu: None,
            failure: None,
            attribution: String::new(),
        });
        let mut posts = Vec::new();
        let mut post = |event: Event, record: &Record| {
            if self.inputs.events.contains(&event) {
                posts.push(Post { name: event.name(), data: serde_json::to_value(record).unwrap_or_default() });
            }
        };
        for event in events {
            post(event, &record);
        }
        for failure in self.failures.drain(..) {
            if let Some(engine) = self.engine.as_mut() {
                engine.report(failure.clone());
            }
            record.failure = Some(failure);
            post(Event::Error, &record);
        }
        (again, posts)
    }

    fn now_ms(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64() * 1000.0
    }
}

/// The JSON text of a prop, which may be a document or a string of one.
fn text(value: Option<&Json>) -> String {
    match value {
        None | Some(Json::Null) => String::new(),
        Some(Json::String(text)) => text.clone(),
        Some(json) => json.to_string(),
    }
}

/// A `camera` prop: what it leaves out is the whole world's.
fn camera(value: Option<&Json>) -> Result<Camera, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else { return Ok(WORLD) };
    let number = |key: &str, default: f64| match value.get(key) {
        None => Ok(default),
        Some(number) => number.as_f64().ok_or_else(|| format!("maplibre: camera: {key} is a number")),
    };
    let center = match value.get("center") {
        None => WORLD.center,
        Some(center) => serde_json::from_value(center.clone())
            .map_err(|_| "maplibre: camera: center is [longitude, latitude]".to_owned())?,
    };
    Ok(Camera { center, zoom: number("zoom", 0.0)?, bearing: number("bearing", 0.0)?, pitch: number("pitch", 0.0)? })
}

/// An `events` prop: names separated by spaces or commas, or a list of them,
/// and every event without one.
fn events(value: Option<&Json>) -> Result<Vec<Event>, String> {
    match value {
        None | Some(Json::Null) => Ok(ALL_EVENTS.to_vec()),
        Some(Json::String(names)) => Event::parse_list(names),
        Some(Json::Array(names)) => {
            let names: Vec<&str> = names.iter().filter_map(Json::as_str).collect();
            Event::parse_list(&names.join(" "))
        }
        Some(_) => Err("maplibre: events: the events are names".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn props_declare_the_map() {
        let mut state = MapState::new();
        state.set_prop("style", Some(&Json::from("https://tiles.openfreemap.org/styles/liberty")));
        state.set_prop("camera", Some(&serde_json::json!({"center": [8.54, 47.37], "zoom": 11})));
        state.set_prop("events", Some(&serde_json::json!(["load", "click"])));
        state.set_prop("options", Some(&serde_json::json!({"maxZoom": 16})));
        assert!(state.failures.is_empty(), "{:?}", state.failures);
        assert_eq!(state.inputs.style, "https://tiles.openfreemap.org/styles/liberty");
        assert_eq!(state.inputs.camera, Camera { center: [8.54, 47.37], zoom: 11.0, bearing: 0.0, pitch: 0.0 });
        assert_eq!(state.inputs.events, vec![Event::Load, Event::Click]);
        assert_eq!(state.inputs.options.max_zoom, Some(16.0));
        state.set_prop("events", None);
        assert_eq!(state.inputs.events, ALL_EVENTS.to_vec());
    }

    #[test]
    fn a_malformed_prop_is_a_failure_the_app_hears_of() {
        let mut state = MapState::new();
        state.set_prop("camera", Some(&serde_json::json!({"zoom": "near"})));
        state.set_prop("events", Some(&Json::from("load nothing")));
        let (again, posts) = state.frame(|_| Err("no target".to_owned()));
        assert!(!again);
        let failures: Vec<_> = posts.iter().map(|post| (post.name, post.data["failure"].as_str())).collect();
        assert_eq!(
            failures,
            [
                ("error", Some("maplibre: camera: zoom is a number")),
                (
                    "error",
                    Some(
                        "maplibre: events: nothing is no event; the map posts load, moveend, click, contextmenu and error"
                    )
                )
            ]
        );
    }

    #[test]
    fn commands_wait_for_the_map() {
        let mut state = MapState::new();
        state.command("flyTo", &serde_json::json!({"center": [8.54, 47.37], "zoom": 14}));
        state.command("jumpTo", &Json::from(3));
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.failures, ["maplibre: jumpTo: its options are an object"]);
    }
}
