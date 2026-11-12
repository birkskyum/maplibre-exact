//! The app's content on an mln-rs style. Each change does the least it can.
//! A GeoJSON source takes new data in place, a layer takes changed paint and
//! layout properties, filter and zoom range in place, and only what the style
//! can't change in place (a source's type or tiles, a layer's type or source,
//! the layers' order) is removed and added again.

use std::collections::HashMap;
use std::sync::Arc;

use mln_conversion::{Convertible, Error as ConversionError};
use mln_style::Style;
use mln_style::conversion::{convert_layer, convert_source};
use mln_style::sources::geojson_source::GeoJSONSource;
use mln_util::ErrorKind;
use mln_util::image::PremultipliedImage;
use serde_json::Value as Json;

use crate::protocol::{Content, Layer};

/// Brings `style` from `applied` to `desired`, with the images decoded so far
/// by their paths, and returns what the style then holds of the content, the
/// errors on the way, and what mln-rs doesn't draw (terrain). What failed to
/// add, or an image still on its way, is left out, so the next change tries
/// it again.
pub(super) fn apply(
    style: &mut Style,
    applied: &Content,
    desired: &Content,
    decoded: &HashMap<String, PremultipliedImage>,
) -> (Content, Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut held = applied.clone();

    // Images go first and come in first, so layers find them.
    held.images.retain(|image| {
        let keep = desired.images.contains(image);
        if !keep {
            style.remove_image(&image.id);
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
        match made {
            Ok(made) => {
                style.add_image(Arc::new(made));
                held.images.push(image.clone());
            }
            Err(e) => errors.push(format!("maplibre: image {}: {}", image.id, e.message)),
        }
    }

    // Sources that change in place, and those that go or are replaced.
    let mut replaced = Vec::new();
    for (id, old) in &applied.sources {
        match desired.source(id) {
            None => replaced.push(id.clone()),
            Some(new) if new == old => {}
            Some(new) if same_but_data(old, new) => match set_geojson_data(style, id, &new["data"]) {
                Ok(()) => set_source(&mut held, id, new),
                Err(error) => {
                    errors.push(error);
                    replaced.push(id.clone());
                }
            },
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
        style.remove_layer(id);
        held.layers.retain(|layer| &layer.id != id);
    }
    for id in &replaced {
        style.remove_source(id);
        held.sources.retain(|(source, _)| source != id);
    }

    // New and replaced sources.
    for (id, source) in &desired.sources {
        if held.source(id).is_some() {
            continue;
        }
        let mut error = ConversionError::default();
        match convert_source(&Convertible::from(source), &mut error, id) {
            Some(converted) => match style.add_source(converted) {
                Ok(()) => held.sources.push((id.clone(), source.clone())),
                Err(e) => errors.push(format!("maplibre: source {id}: {}", e.message)),
            },
            None => errors.push(format!("maplibre: source {id}: {}", error.message)),
        }
    }

    // Layers that stay take their changes in place.
    for new in &desired.layers {
        let Some(index) = held.layers.iter().position(|old| old.id == new.id) else { continue };
        if held.layers[index].json != new.json {
            errors.extend(set_properties(style, &held.layers[index], new));
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
        let under_type = layer.before_type.as_deref().and_then(|kind| first_of_type(style, desired, kind));
        let before = layer.before.clone().or(under_type).or(next);
        let mut error = ConversionError::default();
        match convert_layer(&Convertible::from(&layer.json), &mut error) {
            Some(converted) => match style.add_layer(converted, before.as_deref()) {
                Ok(()) => added.push(layer.id.clone()),
                Err(e) => errors.push(format!("maplibre: layer {}: {}", layer.id, e.message)),
            },
            None => errors.push(format!("maplibre: layer {}: {}", layer.id, error.message)),
        }
    }
    held.layers = desired
        .layers
        .iter()
        .filter(|layer| added.contains(&layer.id) || held.layers.iter().any(|held| held.id == layer.id))
        .cloned()
        .collect();
    held.hit_padding = desired.hit_padding;
    let unsupported = apply_roots(style, &mut held, desired, &mut errors);
    apply_state(style, &mut held, desired, &mut errors);
    (held, errors, unsupported)
}

/// Sets the root properties the content changed over the base style's, and
/// gives back the base style's for those it dropped. Returns what mln-rs
/// doesn't draw, which stays held so that it is reported once.
fn apply_roots(style: &mut Style, held: &mut Content, desired: &Content, errors: &mut Vec<String>) -> Vec<String> {
    let mut unsupported = Vec::new();
    let dropped: Vec<String> =
        held.roots.iter().filter(|(name, _)| desired.root(name).is_none()).map(|(name, _)| name.clone()).collect();
    for name in dropped {
        held.roots.retain(|(root, _)| *root != name);
        let base = serde_json::from_str::<Json>(&style.get_json()).ok().and_then(|base| base.get(&name).cloned());
        if name != "terrain"
            && let Err(e) = style.set_root_property_json(&name, &base.unwrap_or(Json::Null).to_string())
        {
            errors.push(format!("maplibre: {name}: {}", e.message));
        }
    }
    for (name, value) in &desired.roots {
        if held.root(name) == Some(value) {
            continue;
        }
        held.roots.retain(|(root, _)| root != name);
        match style.set_root_property_json(name, &value.to_string()) {
            Ok(()) => held.roots.push((name.clone(), value.clone())),
            Err(e) if e.kind == ErrorKind::Unsupported => {
                unsupported.push(format!("maplibre: {name}: {}", e.message));
                held.roots.push((name.clone(), value.clone()));
            }
            Err(e) => errors.push(format!("maplibre: {name}: {}", e.message)),
        }
    }
    unsupported
}

/// Sets the global state values the content changed, and gives those it
/// dropped back the defaults the style's `state` gives them.
fn apply_state(style: &mut Style, held: &mut Content, desired: &Content, errors: &mut Vec<String>) {
    let dropped: Vec<String> = held
        .state
        .iter()
        .filter(|(name, _)| desired.state_value(name).is_none())
        .map(|(name, _)| name.clone())
        .collect();
    for name in dropped {
        held.state.retain(|(key, _)| *key != name);
        if let Err(e) = style.set_global_state_property_json(&name, "null") {
            errors.push(format!("maplibre: state {name}: {}", e.message));
        }
    }
    for (name, value) in &desired.state {
        if held.state_value(name) == Some(value) {
            continue;
        }
        held.state.retain(|(key, _)| key != name);
        match style.set_global_state_property_json(name, &value.to_string()) {
            Ok(()) => held.state.push((name.clone(), value.clone())),
            Err(e) => errors.push(format!("maplibre: state {name}: {}", e.message)),
        }
    }
}

/// The base style's first layer of type `kind`, which none of the content is.
fn first_of_type(style: &Style, content: &Content, kind: &str) -> Option<String> {
    style
        .get_layers()
        .iter()
        .find(|layer| layer.get_type_info().type_ == kind && !content.layers.iter().any(|c| c.id == layer.get_id()))
        .map(|layer| layer.get_id())
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

fn set_geojson_data(style: &Style, id: &str, data: &Json) -> Result<(), String> {
    let source = style
        .get_source(id)
        .and_then(|source| source.as_any().downcast_ref::<GeoJSONSource>())
        .ok_or_else(|| format!("maplibre: source {id} is no GeoJSON source on the style"))?;
    if let Some(url) = data.as_str() {
        source.set_url(url);
        return Ok(());
    }
    let mut error = ConversionError::default();
    let geojson = mln_conversion::to_geojson(&Convertible::from(data), &mut error)
        .ok_or_else(|| format!("maplibre: source {id}'s data: {}", error.message))?;
    source.set_geojson(&geojson).map_err(|e| format!("maplibre: source {id}'s data: {}", e.message))
}

/// The members of a layer that `set_property` takes; a change to any other
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

/// Sets what changed in `new`'s paint and layout properties, filter and zoom
/// range, and resets what it no longer has.
fn set_properties(style: &mut Style, old: &Layer, new: &Layer) -> Vec<String> {
    let mut changes: Vec<(String, Json)> = Vec::new();
    for group in ["paint", "layout"] {
        let empty = serde_json::Map::new();
        let (before, after) = (
            old.json.get(group).and_then(Json::as_object).unwrap_or(&empty),
            new.json.get(group).and_then(Json::as_object).unwrap_or(&empty),
        );
        for name in before.keys().chain(after.keys().filter(|name| !before.contains_key(*name))) {
            let value = after.get(name).cloned().unwrap_or(Json::Null);
            if before.get(name) != after.get(name) {
                changes.push((name.clone(), value));
            }
        }
    }
    for member in ["filter", "minzoom", "maxzoom"] {
        if old.json.get(member) != new.json.get(member) {
            changes.push((member.to_owned(), new.json.get(member).cloned().unwrap_or(Json::Null)));
        }
    }
    let Some(mut layer) = style.get_layer_mut(&new.id) else {
        return vec![format!("maplibre: layer {} is gone from the style", new.id)];
    };
    changes
        .iter()
        .filter_map(|(name, value)| {
            layer
                .set_property(name, &Convertible::from(value), None)
                .map(|error| format!("maplibre: layer {}'s {name}: {}", new.id, error.message))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"{"version":8,"sources":{},"layers":[
        {"id":"water","type":"background"},{"id":"labels","type":"background"}]}"#;

    fn style() -> Style {
        let mut style = Style::new(1.0);
        style.load_json(BASE);
        style
    }

    fn content(text: &str) -> Content {
        Content::parse(text).unwrap_or_else(|e| panic!("{e}"))
    }

    fn layers(style: &Style) -> Vec<String> {
        style.get_layers().iter().map(|layer| layer.get_id()).collect()
    }

    fn paint(style: &Style, layer: &str, name: &str) -> Json {
        let layer = style.get_layer(layer).unwrap_or_else(|| panic!("no layer {layer}"));
        mln_conversion::value_to_json(&layer.serialize())["paint"][name].clone()
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
        let mut style = style();
        let (held, errors, _) = apply(&mut style, &Content::default(), &content(&dots("red")), &HashMap::new());
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(layers(&style), ["water", "halo", "labels", "dots"]);
        assert!(style.get_source("points").is_some());
        assert_eq!(held.layers.len(), 2);
    }

    #[test]
    fn a_layer_can_draw_under_the_first_base_layer_of_a_type() {
        let mut style = Style::new(1.0);
        style.load_json(
            r#"{"version":8,"sources":{"points":{"type":"geojson","data":{"type":"FeatureCollection","features":[]}}},
                "layers":[{"id":"water","type":"background"},
                          {"id":"road-names","type":"symbol","source":"points"},
                          {"id":"city-names","type":"symbol","source":"points"}]}"#,
        );
        let under_labels = r#"{"layers":[{"id":"trail","type":"line","source":"points","beforeType":"symbol"}]}"#;
        let (_, errors, _) = apply(&mut style, &Content::default(), &content(under_labels), &HashMap::new());
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(layers(&style), ["water", "trail", "road-names", "city-names"]);
    }

    #[test]
    fn a_changed_paint_property_is_set_on_the_layer_that_is_there() {
        let mut style = style();
        let (held, _, _) = apply(&mut style, &Content::default(), &content(&dots("red")), &HashMap::new());
        let (held, errors, _) = apply(&mut style, &held, &content(&dots("blue")), &HashMap::new());
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(paint(&style, "dots", "circle-color"), serde_json::json!(["rgba", 0.0, 0.0, 255.0, 1.0]));
        assert_eq!(layers(&style), ["water", "halo", "labels", "dots"]);
        assert_eq!(held, content(&dots("blue")));
    }

    #[test]
    fn a_removed_property_goes_back_to_its_default() {
        let mut style = style();
        let (held, _, _) = apply(&mut style, &Content::default(), &content(&dots("red")), &HashMap::new());
        let without = r#"{"sources":{"points":POINTS},"layers":[
            {"id":"halo","type":"circle","source":"points","beforeId":"labels"},
            {"id":"dots","type":"circle","source":"points"}]}"#
            .replace("POINTS", POINTS);
        apply(&mut style, &held, &content(&without), &HashMap::new());
        assert_eq!(paint(&style, "dots", "circle-color"), Json::Null);
    }

    #[test]
    fn reordered_layers_are_added_again_in_the_new_order() {
        let mut style = style();
        let two = |first: &str, second: &str| {
            content(&format!(
                r#"{{"sources":{{"points":{POINTS}}},"layers":[
                    {{"id":"{first}","type":"circle","source":"points"}},
                    {{"id":"{second}","type":"circle","source":"points"}}]}}"#
            ))
        };
        let (held, _, _) = apply(&mut style, &Content::default(), &two("a", "b"), &HashMap::new());
        assert_eq!(layers(&style), ["water", "labels", "a", "b"]);
        apply(&mut style, &held, &two("b", "a"), &HashMap::new());
        assert_eq!(layers(&style), ["water", "labels", "b", "a"]);
    }

    #[test]
    fn content_that_goes_leaves_the_style_as_it_was() {
        let mut style = style();
        let (held, _, _) = apply(&mut style, &Content::default(), &content(&dots("red")), &HashMap::new());
        let (held, errors, _) = apply(&mut style, &held, &Content::default(), &HashMap::new());
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(layers(&style), ["water", "labels"]);
        assert!(style.get_source("points").is_none());
        assert_eq!(held, Content::default());
    }

    #[test]
    fn an_image_comes_in_once_it_is_decoded_and_goes_with_the_content() {
        let mut style = style();
        let pin = content(r#"{"images":{"pin":"assets/pin.png"}}"#);
        let (held, _, _) = apply(&mut style, &Content::default(), &pin, &HashMap::new());
        assert!(style.get_image("pin").is_none());
        let decoded =
            HashMap::from([("assets/pin.png".to_owned(), PremultipliedImage::new(mln_util::size::Size::new(4, 4)))]);
        let (held, errors, _) = apply(&mut style, &held, &pin, &decoded);
        assert_eq!(errors, Vec::<String>::new());
        assert!(style.get_image("pin").is_some());
        apply(&mut style, &held, &Content::default(), &decoded);
        assert!(style.get_image("pin").is_none());
    }

    #[test]
    fn a_layer_that_fails_is_left_out_and_reported() {
        let mut style = style();
        let bad = r#"{"layers":[{"id":"x","type":"blob"}]}"#;
        let (held, errors, _) = apply(&mut style, &Content::default(), &content(bad), &HashMap::new());
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(held.layers.is_empty());
    }

    #[test]
    fn root_properties_go_over_the_base_styles_and_give_it_back() {
        let mut style = Style::new(1.0);
        style.load_json(r#"{"version":8,"sources":{},"layers":[],"projection":{"type":"mercator"}}"#);
        let globe = content(r#"{"projection":{"type":"globe"},"terrain":{"source":"dem"}}"#);
        let (held, errors, unsupported) = apply(&mut style, &Content::default(), &globe, &HashMap::new());
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(unsupported.len(), 1, "terrain: {unsupported:?}");
        assert_eq!(style.root_property_json("projection").unwrap().as_deref(), Some(r#"{"type":"globe"}"#));
        // Terrain is reported once, not again on the next change.
        let (held, _, unsupported) = apply(&mut style, &held, &globe, &HashMap::new());
        assert!(unsupported.is_empty());
        apply(&mut style, &held, &Content::default(), &HashMap::new());
        assert_eq!(style.root_property_json("projection").unwrap().as_deref(), Some(r#"{"type":"mercator"}"#));
    }

    #[test]
    fn state_sets_global_state_and_dropping_it_gives_back_the_default() {
        let mut style = Style::new(1.0);
        style.load_json(r#"{"version":8,"sources":{},"layers":[],"state":{"chosen":{"default":""}}}"#);
        let (held, errors, _) =
            apply(&mut style, &Content::default(), &content(r#"{"state":{"chosen":"hb"}}"#), &HashMap::new());
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(style.global_state_json(), r#"{"chosen":"hb"}"#);
        apply(&mut style, &held, &Content::default(), &HashMap::new());
        assert_eq!(style.global_state_json(), r#"{"chosen":""}"#);
    }
}
