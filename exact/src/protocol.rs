//! What an Exact app gives a map: the canvas surface's named arguments
//! ([`ARGUMENTS`]), bound into the map the app declares ([`Inputs`]). The rest
//! of the protocol is maplibre-rust-ui's, which it re-exports: commands come in
//! through Contract's `postMessage(text, "maplibre")`, the names of the events
//! the `events` argument lists go to the canvas's `message=` action, and the
//! app reads the record with `exactSurface("maplibre")` as a shape of its own.

use exact_gpu::Value;
pub use maplibre_rust_ui::model::*;

/// The surface's arguments in bind order.
pub const ARGUMENTS: [&str; 9] =
    ["style", "longitude", "latitude", "zoom", "bearing", "pitch", "content", "options", "events"];

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
        ("options", Value::from("")),
        ("events", Value::from("")),
    ]
}

/// The map an app declares with the arguments in [`ARGUMENTS`] order.
///
/// # Errors
///
/// The message names the argument that is missing, of the wrong type or
/// malformed.
pub fn inputs(values: &[Value]) -> Result<Inputs, String> {
    let [style, longitude, latitude, zoom, bearing, pitch, content, options, events] = values else {
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
        options: Options::parse(options.as_str().ok_or("maplibre: options must be text")?)?,
        events: Event::parse_list(events.as_str().ok_or("maplibre: events must be text")?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arguments_bind_in_order() {
        let mut values: Vec<Value> = argument_defaults().into_iter().map(|(_, v)| v).collect();
        values[0] = Value::from("https://example.com/style.json");
        values[3] = Value::Number(4.0);
        values[8] = Value::from("click");
        let inputs = inputs(&values).unwrap();
        assert_eq!(inputs.camera.zoom, 4.0);
        assert_eq!(inputs.events, vec![Event::Click]);
    }

    #[test]
    fn an_empty_style_is_refused() {
        let values: Vec<Value> = argument_defaults().into_iter().map(|(_, v)| v).collect();
        assert!(inputs(&values).is_err());
    }

}
