//! The app's content on the map's style, sent as the map's style commands.
//! Each change does the least it can. A GeoJSON source takes new data in
//! place, a layer takes changed paint and layout properties, filter and zoom
//! range in place, and only what the style can't change in place (a source's
//! type or tiles, a layer's type or source, the layers' order) is removed and
//! added again.
//!
//! The commands run in order on the map's thread and report how they went
//! later, so what the map holds of the content is what was sent. A command
//! that fails takes its part back out, so the next change tries it again.

use std::collections::HashMap;

use mln_runtime::{Applied, Completion, Error, ErrorKind, Map};
use mln_util::image::PremultipliedImage;
use serde_json::Value as Json;

use crate::protocol::{Content, Layer};

/// What content needs of the loaded style itself: its root properties, which
/// come back when the content drops its own, and its layers by type, which a
/// content layer can draw under.
#[derive(Debug, Default)]
pub(super) struct Base {
    roots: serde_json::Map<String, Json>,
    layers: Vec<(String, String)>,
}

impl Base {
    /// The base of a loaded style's JSON, before any content is on it.
    pub(super) fn parse(style: &str) -> Base {
        let Ok(Json::Object(mut style)) = serde_json::from_str::<Json>(style) else { return Base::default() };
        let layers = match style.remove("layers") {
            Some(Json::Array(layers)) => layers
                .iter()
                .filter_map(|layer| Some((layer.get("id")?.as_str()?.to_owned(), layer.get("type")?.as_str()?.to_owned())))
                .collect(),
            _ => Vec::new(),
        };
        Base { roots: style, layers }
    }

    /// The first of the style's layers of type `kind` that none of the
    /// content is.
    fn first_of_type(&self, content: &Content, kind: &str) -> Option<String> {
        self.layers
            .iter()
            .find(|(id, type_)| type_ == kind && !content.layers.iter().any(|layer| &layer.id == id))
            .map(|(id, _)| id.clone())
    }
}

/// A command sent for the content, and what it takes back out of what the
/// map holds if it fails.
pub(super) struct Sent {
    pub(super) completion: Completion<Applied>,
    pub(super) what: String,
    pub(super) undo: Undo,
}

pub(super) enum Undo {
    Nothing,
    Image(String),
    Source(String),
    Layer(String),
    Root(String),
    State(String),
}

impl Undo {
    /// Takes what failed out of `held`. A root property that the engine
    /// doesn't draw stays, so that it is reported once.
    pub(super) fn apply(&self, held: &mut Content, error: &Error) {
        match self {
            Undo::Nothing => {}
            Undo::Image(id) => held.images.retain(|image| &image.id != id),
            Undo::Source(id) => held.sources.retain(|(source, _)| source != id),
            Undo::Layer(id) => held.layers.retain(|layer| &layer.id != id),
            Undo::Root(name) if error.kind != ErrorKind::Unsupported => held.roots.retain(|(root, _)| root != name),
            Undo::Root(_) => {}
            Undo::State(name) => held.state.retain(|(key, _)| key != name),
        }
    }
}

/// The commands of one change, in the order they run.
struct Commands<'a> {
    map: &'a Map,
    sent: Vec<Sent>,
}

impl Commands<'_> {
    fn send(&mut self, what: String, undo: Undo, command: Result<Completion<Applied>, Error>) {
        let completion = command.unwrap_or_else(|error| Completion::ready(Err(error)));
        self.sent.push(Sent { completion, what, undo });
    }
}

/// Brings the map's style from `applied` to `desired`, with the images
/// decoded so far by their paths, on a style whose base is `base`. Returns
/// what the style then holds of the content, and the commands sent, whose
/// failures [`Undo`] takes back out. An image still on its way is left out,
/// so the next change brings it in.
pub(super) fn apply(
    map: &Map,
    base: &Base,
    applied: &Content,
    desired: &Content,
    decoded: &HashMap<String, PremultipliedImage>,
) -> (Content, Vec<Sent>) {
    let mut commands = Commands { map, sent: Vec::new() };
    let mut held = applied.clone();

    // Images go first and come in first, so layers find them.
    held.images.retain(|image| {
        let keep = desired.images.contains(image);
        if !keep {
            commands.send(format!("image {}", image.id), Undo::Nothing, map.remove_style_image(&image.id));
        }
        keep
    });
    for image in &desired.images {
        if held.images.contains(image) {
            continue;
        }
        let Some(pixels) = decoded.get(&image.src) else { continue };
        let made = mln_style::image::Impl::new(
            image.id.clone(),
            pixels.clone(),
            image.pixel_ratio,
            image.sdf,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        );
        let what = format!("image {}", image.id);
        commands.send(what, Undo::Image(image.id.clone()), made.and_then(|made| map.set_style_image(made)));
        held.images.push(image.clone());
    }

    // Sources that change in place, and those that go or are replaced.
    let mut replaced = Vec::new();
    for (id, old) in &applied.sources {
        match desired.source(id) {
            None => replaced.push(id.clone()),
            Some(new) if new == old => {}
            Some(new) if same_but_data(old, new) => {
                let data = &new["data"];
                let command = match data.as_str() {
                    Some(url) => map.set_geojson_source_url(id, url),
                    None => map.set_geojson_source_data(id, data.to_string()),
                };
                commands.send(format!("source {id}'s data"), Undo::Nothing, command);
                set_source(&mut held, id, new);
            }
            Some(_) => replaced.push(id.clone()),
        }
    }

    // Layers that go, because they left the content, moved, or can't change in place.
    let reordered = order(&applied.layers, desired) != order(&desired.layers, applied);
    let removed: Vec<String> = applied
        .layers
        .iter()
        .filter(|old| {
            reordered
                || replaced.contains(&source_of(&old.json))
                || desired.layers.iter().find(|new| new.id == old.id).is_none_or(|new| !in_place(old, new))
        })
        .map(|old| old.id.clone())
        .collect();
    for id in &removed {
        commands.send(format!("layer {id}"), Undo::Nothing, map.remove_style_layer(id));
        held.layers.retain(|layer| &layer.id != id);
    }
    for id in &replaced {
        commands.send(format!("source {id}"), Undo::Nothing, map.remove_style_source(id));
        held.sources.retain(|(source, _)| source != id);
    }

    // New and replaced sources.
    for (id, source) in &desired.sources {
        if held.source(id).is_some() {
            continue;
        }
        commands.send(format!("source {id}"), Undo::Source(id.clone()), map.add_style_source_json(id, source.to_string()));
        held.sources.push((id.clone(), source.clone()));
    }

    // Layers that stay take their changes in place.
    for new in &desired.layers {
        let Some(index) = held.layers.iter().position(|old| old.id == new.id) else { continue };
        if held.layers[index].json != new.json {
            for (name, value) in changes(&held.layers[index], new) {
                let command = map.set_style_layer_property(&new.id, &name, value.to_string());
                commands.send(format!("layer {}'s {name}", new.id), Undo::Nothing, command);
            }
            held.layers[index] = new.clone();
        }
    }

    // New layers, from the top down, each under the next one on the style.
    let mut added = Vec::new();
    for (index, layer) in desired.layers.iter().enumerate().rev() {
        if held.layers.iter().any(|held| held.id == layer.id) {
            continue;
        }
        let next = desired.layers[index + 1..]
            .iter()
            .find(|above| held.layers.iter().any(|held| held.id == above.id) || added.contains(&above.id))
            .map(|above| above.id.clone());
        let under_type = layer.before_type.as_deref().and_then(|kind| base.first_of_type(desired, kind));
        let before = layer.before.clone().or(under_type).or(next);
        let command = map.add_style_layer_json(layer.json.to_string(), before);
        commands.send(format!("layer {}", layer.id), Undo::Layer(layer.id.clone()), command);
        added.push(layer.id.clone());
    }
    held.layers = desired
        .layers
        .iter()
        .filter(|layer| added.contains(&layer.id) || held.layers.iter().any(|held| held.id == layer.id))
        .cloned()
        .collect();
    held.hit_padding = desired.hit_padding;
    apply_roots(&mut commands, base, &mut held, desired);
    apply_state(&mut commands, &mut held, desired);
    (held, commands.sent)
}

/// Sets the root properties the content changed over the base style's, and
/// gives back the base style's for those it dropped.
fn apply_roots(commands: &mut Commands<'_>, base: &Base, held: &mut Content, desired: &Content) {
    let dropped: Vec<String> =
        held.roots.iter().filter(|(name, _)| desired.root(name).is_none()).map(|(name, _)| name.clone()).collect();
    for name in dropped {
        held.roots.retain(|(root, _)| *root != name);
        if name != "terrain" {
            let value = base.roots.get(&name).cloned().unwrap_or(Json::Null);
            let command = commands.map.set_style_root_property(&name, value.to_string());
            commands.send(name, Undo::Nothing, command);
        }
    }
    for (name, value) in &desired.roots {
        if held.root(name) == Some(value) {
            continue;
        }
        held.roots.retain(|(root, _)| root != name);
        let command = commands.map.set_style_root_property(name, value.to_string());
        commands.send(name.clone(), Undo::Root(name.clone()), command);
        held.roots.push((name.clone(), value.clone()));
    }
}

/// Sets the global state values the content changed, and gives those it
/// dropped back the defaults the style's `state` gives them.
fn apply_state(commands: &mut Commands<'_>, held: &mut Content, desired: &Content) {
    let dropped: Vec<String> = held
        .state
        .iter()
        .filter(|(name, _)| desired.state_value(name).is_none())
        .map(|(name, _)| name.clone())
        .collect();
    for name in dropped {
        held.state.retain(|(key, _)| *key != name);
        let command = commands.map.set_global_state_property(&name, "null");
        commands.send(format!("state {name}"), Undo::Nothing, command);
    }
    for (name, value) in &desired.state {
        if held.state_value(name) == Some(value) {
            continue;
        }
        held.state.retain(|(key, _)| key != name);
        let command = commands.map.set_global_state_property(name, value.to_string());
        commands.send(format!("state {name}"), Undo::State(name.clone()), command);
        held.state.push((name.clone(), value.clone()));
    }
}

/// The ids of `layers` that `other` has too, in `layers`' order.
fn order<'a>(layers: &'a [Layer], other: &Content) -> Vec<&'a str> {
    layers.iter().filter(|layer| other.layers.iter().any(|o| o.id == layer.id)).map(|layer| layer.id.as_str()).collect()
}

fn source_of(layer: &Json) -> String {
    layer.get("source").and_then(Json::as_str).unwrap_or_default().to_owned()
}

fn set_source(content: &mut Content, id: &str, source: &Json) {
    if let Some((_, held)) = content.sources.iter_mut().find(|(held, _)| held == id) {
        *held = source.clone();
    }
}

/// Whether two GeoJSON sources differ in their data alone.
fn same_but_data(old: &Json, new: &Json) -> bool {
    let without_data = |source: &Json| {
        let mut source = source.clone();
        if let Json::Object(object) = &mut source {
            object.remove("data");
        }
        source
    };
    old.get("type").and_then(Json::as_str) == Some("geojson") && without_data(old) == without_data(new)
}

/// The members of a layer that the map sets in place; a change to any other
/// one makes a new layer.
const SETTABLE: [&str; 5] = ["paint", "layout", "filter", "minzoom", "maxzoom"];

fn in_place(old: &Layer, new: &Layer) -> bool {
    let rest = |layer: &Layer| {
        let mut json = layer.json.clone();
        if let Json::Object(object) = &mut json {
            for member in SETTABLE {
                object.remove(member);
            }
        }
        json
    };
    old.before == new.before && old.before_type == new.before_type && rest(old) == rest(new)
}

/// What changed in `new`'s paint and layout properties, filter and zoom
/// range, with `null` for what it no longer has, which resets it.
fn changes(old: &Layer, new: &Layer) -> Vec<(String, Json)> {
    let mut changes = Vec::new();
    for group in ["paint", "layout"] {
        let empty = serde_json::Map::new();
        let (before, after) = (
            old.json.get(group).and_then(Json::as_object).unwrap_or(&empty),
            new.json.get(group).and_then(Json::as_object).unwrap_or(&empty),
        );
        for name in before.keys().chain(after.keys().filter(|name| !before.contains_key(*name))) {
            if before.get(name) != after.get(name) {
                changes.push((name.clone(), after.get(name).cloned().unwrap_or(Json::Null)));
            }
        }
    }
    for member in ["filter", "minzoom", "maxzoom"] {
        if old.json.get(member) != new.json.get(member) {
            changes.push((member.to_owned(), new.json.get(member).cloned().unwrap_or(Json::Null)));
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use mln_runtime::{MapOptions, Runtime, RuntimeOptions};

    use super::*;

    const BASE: &str = r#"{"version":8,"sources":{},"layers":[
        {"id":"water","type":"background"},{"id":"labels","type":"background"}]}"#;

    /// A map of the style `json`, and its base.
    fn map(json: &str) -> (Runtime, Map, Base) {
        let runtime = Runtime::new(&RuntimeOptions::default()).unwrap();
        let (map, created) = runtime.create_map(MapOptions::default()).unwrap();
        created.wait().unwrap();
        map.set_style_json(json).unwrap().wait().unwrap();
        let base = Base::parse(&map.loaded_style_json().unwrap().wait().unwrap());
        (runtime, map, base)
    }

    /// Applies `desired` and waits for its commands. Returns what the map
    /// holds of it and the errors, each with what failed.
    fn change(
        map: &Map,
        base: &Base,
        applied: &Content,
        desired: &Content,
        decoded: &HashMap<String, PremultipliedImage>,
    ) -> (Content, Vec<(String, ErrorKind)>) {
        let (mut held, sent) = apply(map, base, applied, desired, decoded);
        let mut errors = Vec::new();
        for sent in sent {
            if let Err(error) = sent.completion.wait() {
                sent.undo.apply(&mut held, &error);
                errors.push((sent.what, error.kind));
            }
        }
        (held, errors)
    }

    fn content(text: &str) -> Content {
        Content::parse(text).unwrap_or_else(|e| panic!("{e}"))
    }

    fn layers(map: &Map) -> Vec<String> {
        map.style_layers().unwrap().wait().unwrap().into_iter().map(|layer| layer.id).collect()
    }

    fn paint(map: &Map, layer: &str, name: &str) -> Json {
        let json = map.style_layer_json(layer).unwrap().wait().unwrap().unwrap_or_else(|| panic!("no layer {layer}"));
        serde_json::from_str::<Json>(&json).unwrap()["paint"][name].clone()
    }

    const POINTS: &str = r#"{"type":"geojson","data":{"type":"FeatureCollection","features":[]}}"#;

    fn dots(color: &str) -> String {
        format!(
            r#"{{"sources":{{"points":{POINTS}}},"layers":[
                {{"id":"halo","type":"circle","source":"points","beforeId":"labels"}},
                {{"id":"dots","type":"circle","source":"points","paint":{{"circle-color":"{color}"}}}}]}}"#
        )
    }

    #[test]
    fn content_draws_over_the_base_style_in_order_unless_it_names_a_layer_to_draw_under() {
        let (_runtime, map, base) = map(BASE);
        let (held, errors) = change(&map, &base, &Content::default(), &content(&dots("red")), &HashMap::new());
        assert_eq!(errors, Vec::new());
        assert_eq!(layers(&map), ["water", "halo", "labels", "dots"]);
        assert!(map.style_source("points").unwrap().wait().unwrap().is_some());
        assert_eq!(held.layers.len(), 2);
    }

    #[test]
    fn a_layer_can_draw_under_the_first_base_layer_of_a_type() {
        let (_runtime, map, base) = map(
            r#"{"version":8,"sources":{"points":{"type":"geojson","data":{"type":"FeatureCollection","features":[]}}},
                "layers":[{"id":"water","type":"background"},
                          {"id":"road-names","type":"symbol","source":"points"},
                          {"id":"city-names","type":"symbol","source":"points"}]}"#,
        );
        let under_labels = r#"{"layers":[{"id":"trail","type":"line","source":"points","beforeType":"symbol"}]}"#;
        let (_, errors) = change(&map, &base, &Content::default(), &content(under_labels), &HashMap::new());
        assert_eq!(errors, Vec::new());
        assert_eq!(layers(&map), ["water", "trail", "road-names", "city-names"]);
    }

    #[test]
    fn a_changed_paint_property_is_set_on_the_layer_that_is_there() {
        let (_runtime, map, base) = map(BASE);
        let (held, _) = change(&map, &base, &Content::default(), &content(&dots("red")), &HashMap::new());
        let (held, errors) = change(&map, &base, &held, &content(&dots("blue")), &HashMap::new());
        assert_eq!(errors, Vec::new());
        assert_eq!(paint(&map, "dots", "circle-color"), serde_json::json!(["rgba", 0.0, 0.0, 255.0, 1.0]));
        assert_eq!(layers(&map), ["water", "halo", "labels", "dots"]);
        assert_eq!(held, content(&dots("blue")));
    }

    #[test]
    fn a_removed_property_goes_back_to_its_default() {
        let (_runtime, map, base) = map(BASE);
        let (held, _) = change(&map, &base, &Content::default(), &content(&dots("red")), &HashMap::new());
        let without = r#"{"sources":{"points":POINTS},"layers":[
            {"id":"halo","type":"circle","source":"points","beforeId":"labels"},
            {"id":"dots","type":"circle","source":"points"}]}"#
            .replace("POINTS", POINTS);
        let (_, errors) = change(&map, &base, &held, &content(&without), &HashMap::new());
        assert_eq!(errors, Vec::new());
        assert_eq!(paint(&map, "dots", "circle-color"), Json::Null);
    }

    #[test]
    fn reordered_layers_are_added_again_in_the_new_order() {
        let (_runtime, map, base) = map(BASE);
        let two = |first: &str, second: &str| {
            content(&format!(
                r#"{{"sources":{{"points":{POINTS}}},"layers":[
                    {{"id":"{first}","type":"circle","source":"points"}},
                    {{"id":"{second}","type":"circle","source":"points"}}]}}"#
            ))
        };
        let (held, _) = change(&map, &base, &Content::default(), &two("a", "b"), &HashMap::new());
        assert_eq!(layers(&map), ["water", "labels", "a", "b"]);
        change(&map, &base, &held, &two("b", "a"), &HashMap::new());
        assert_eq!(layers(&map), ["water", "labels", "b", "a"]);
    }

    #[test]
    fn content_that_goes_leaves_the_style_as_it_was() {
        let (_runtime, map, base) = map(BASE);
        let (held, _) = change(&map, &base, &Content::default(), &content(&dots("red")), &HashMap::new());
        let (held, errors) = change(&map, &base, &held, &Content::default(), &HashMap::new());
        assert_eq!(errors, Vec::new());
        assert_eq!(layers(&map), ["water", "labels"]);
        assert!(map.style_source("points").unwrap().wait().unwrap().is_none());
        assert_eq!(held, Content::default());
    }

    #[test]
    fn an_image_comes_in_once_it_is_decoded_and_goes_with_the_content() {
        let (_runtime, map, base) = map(BASE);
        let image = |map: &Map| map.style_image("pin").unwrap().wait().unwrap();
        let pin = content(r#"{"images":{"pin":"assets/pin.png"}}"#);
        let (held, _) = change(&map, &base, &Content::default(), &pin, &HashMap::new());
        assert!(image(&map).is_none());
        let decoded =
            HashMap::from([("assets/pin.png".to_owned(), PremultipliedImage::new(mln_util::size::Size::new(4, 4)))]);
        let (held, errors) = change(&map, &base, &held, &pin, &decoded);
        assert_eq!(errors, Vec::new());
        assert!(image(&map).is_some());
        change(&map, &base, &held, &Content::default(), &decoded);
        assert!(image(&map).is_none());
    }

    #[test]
    fn a_layer_that_fails_is_left_out_and_reported() {
        let (_runtime, map, base) = map(BASE);
        let bad = r#"{"layers":[{"id":"x","type":"blob"}]}"#;
        let (held, errors) = change(&map, &base, &Content::default(), &content(bad), &HashMap::new());
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(held.layers.is_empty());
    }

    #[test]
    fn root_properties_go_over_the_base_styles_and_give_it_back() {
        let (_runtime, map, base) =
            map(r#"{"version":8,"sources":{},"layers":[],"projection":{"type":"mercator"}}"#);
        let projection = |map: &Map| map.style_root_property("projection").unwrap().wait().unwrap();
        let globe = content(r#"{"projection":{"type":"globe"},"terrain":{"source":"dem"}}"#);
        let (held, errors) = change(&map, &base, &Content::default(), &globe, &HashMap::new());
        assert_eq!(errors, [("terrain".to_owned(), ErrorKind::Unsupported)]);
        assert_eq!(projection(&map).as_deref(), Some(r#"{"type":"globe"}"#));
        // Terrain stays held, so it is reported once, not again on the next change.
        let (held, errors) = change(&map, &base, &held, &globe, &HashMap::new());
        assert_eq!(errors, Vec::new());
        change(&map, &base, &held, &Content::default(), &HashMap::new());
        assert_eq!(projection(&map).as_deref(), Some(r#"{"type":"mercator"}"#));
    }

    #[test]
    fn state_sets_global_state_and_dropping_it_gives_back_the_default() {
        let (_runtime, map, base) =
            map(r#"{"version":8,"sources":{},"layers":[],"state":{"chosen":{"default":""}}}"#);
        let state = |map: &Map| map.global_state().unwrap().wait().unwrap();
        let (held, errors) =
            change(&map, &base, &Content::default(), &content(r#"{"state":{"chosen":"hb"}}"#), &HashMap::new());
        assert_eq!(errors, Vec::new());
        assert_eq!(state(&map), r#"{"chosen":"hb"}"#);
        change(&map, &base, &held, &Content::default(), &HashMap::new());
        assert_eq!(state(&map), r#"{"chosen":""}"#);
    }
}
