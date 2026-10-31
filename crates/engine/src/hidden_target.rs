//! Photoshop refuses to paint on or move a hidden layer (#571): the painting and retouching
//! tools, the Paint Bucket, the Gradient and the Move tool stop with "the target layer is hidden"
//! instead of changing pixels nobody can see. So do Cut, Clear and the fills, and the filters and
//! adjustments refuse with their own alert (#10476); Copy still copies. A layer inside a hidden
//! group is hidden too. An alpha channel or Quick Mask target has no layer, so it is never refused.

use photocraft_doc::{Document, LayerId};
use serde_json::Value;

use crate::Session;

/// Photoshop's alert text.
pub const HIDDEN: &str = "Could not complete your request because the target layer is hidden.";

/// Photoshop's alert for a filter or an Image › Adjustments command on a hidden layer.
pub const HIDDEN_FILTER: &str = "You cannot apply a filter or an adjustment to a hidden layer.";

/// Commands that paint on, or move, the target layer.
const GATED: &[&str] = &[
    "paint.stroke",
    "paint.pencil",
    "paint.mixerBrush",
    "paint.colorReplacement",
    "paint.bucket",
    "paint.gradient",
    "paint.magicEraser",
    "paint.backgroundEraser",
    "paint.cloneStamp",
    "paint.healingBrush",
    "paint.spotHealing",
    "paint.patch",
    "paint.dodge",
    "paint.burn",
    "paint.sponge",
    "paint.blur",
    "paint.sharpen",
    "paint.smudge",
    "paint.historyBrush",
    "paint.artHistoryBrush",
    "layer.translate",
    // Edit › Cut, Clear and the fills (#10476).
    "edit.cut",
    "edit.clear",
    "edit.fill",
    "edit.fillForeground",
    "edit.fillBackground",
    "edit.fillForegroundPreserve",
    "edit.fillBackgroundPreserve",
    "edit.fillHistory",
];

/// Filters and adjustments that change the target layer, refused with [`HIDDEN_FILTER`]: every
/// `filter.*` and `image.adjustments.*` command but the catalogue queries, the render filters that
/// make a new layer, and Convert for Smart Filters.
fn filters_the_layer(id: &str, params: &Value) -> bool {
    (id.starts_with("filter.") || id.starts_with("image.adjustments."))
        && !id.ends_with(".list")
        && params.get("list").and_then(Value::as_bool) != Some(true)
        && !matches!(id, "filter.render.flame" | "filter.render.tree" | "filter.render.pictureFrame" | "filter.convertForSmartFilters")
}

/// Is the layer shown: it and every group around it visible.
fn shown(doc: &Document, id: LayerId) -> bool {
    doc.path_of(id).is_some_and(|path| (1..=path.len()).all(|n| path.get(..n).and_then(|p| doc.layer_at(p)).is_some_and(|l| l.visible)))
}

/// Why command `id` with `params` must not run: its target layer is hidden. `None` for other
/// commands, channel targets, and visible layers.
pub fn refusal(s: &Session, id: &str, params: &Value) -> Option<&'static str> {
    let why = if GATED.contains(&id) {
        HIDDEN
    } else if filters_the_layer(id, params) {
        HIDDEN_FILTER
    } else {
        return None;
    };
    let params = crate::channel_cmds::inject_target(s, id, params.clone());
    if crate::channel_cmds::is_channel_target(&params) {
        return None;
    }
    let st = s.active()?;
    let layer = params.get("layer").and_then(Value::as_u64).map(LayerId).or(st.active_layer)?;
    (st.doc.layer(layer).is_some() && !shown(&st.doc, layer)).then_some(why)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{HIDDEN, HIDDEN_FILTER};
    use crate::Session;

    fn session() -> (Session, u64) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
        let id = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        (s, id)
    }

    fn pixel(s: &mut Session) -> serde_json::Value {
        s.execute("document.pixel", json!({"x": 10, "y": 10})).unwrap()
    }

    #[test]
    fn painting_or_moving_a_hidden_layer_is_refused() {
        let (mut s, id) = session();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        let steps = s.active().unwrap().history.entries().len();
        for (cmd, p) in [
            ("paint.stroke", json!({"points": [[10, 10], [20, 20]], "color": "#ff0000"})),
            ("paint.bucket", json!({"x": 10, "y": 10})),
            ("paint.gradient", json!({"from": [0, 0], "to": [60, 60]})),
            ("layer.translate", json!({"dx": 5, "dy": 0})),
        ] {
            let e = s.execute(cmd, p).unwrap_err().to_string();
            assert!(e.contains(HIDDEN), "{cmd}: {e}");
        }
        assert_eq!(s.active().unwrap().history.entries().len(), steps, "nothing changed");
        // Shown again, it paints.
        s.execute("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
        s.execute("paint.stroke", json!({"points": [[10, 10], [12, 12]], "color": "#ff0000", "size": 9})).unwrap();
        assert_ne!(pixel(&mut s), json!([1.0, 1.0, 1.0, 1.0]));
    }

    /// #10476: Photoshop 2025 with a hidden layer selected and a selection: Copy and Copy Merged
    /// run; Cut, Clear and Fill are refused; Gaussian Blur alerts "You cannot apply a filter or an
    /// adjustment to a hidden layer". All of them ran on the hidden layer.
    #[test]
    fn cut_clear_fill_filters_and_adjustments_refuse_a_hidden_layer() {
        let (mut s, id) = session();
        s.execute("edit.fill", json!({"contents": "color", "color": "#0000c8"})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        let steps = s.active().unwrap().history.entries().len();
        for (cmd, p, why) in [
            ("edit.cut", json!({}), HIDDEN),
            ("edit.clear", json!({}), HIDDEN),
            ("edit.fill", json!({"contents": "color", "color": "#00ff00"}), HIDDEN),
            ("edit.fillForeground", json!({}), HIDDEN),
            ("filter.blur.gaussianBlur", json!({"radius": 2}), HIDDEN_FILTER),
            ("image.adjustments.invert", json!({}), HIDDEN_FILTER),
        ] {
            let e = s.execute(cmd, p).unwrap_err().to_string();
            assert!(e.contains(why), "{cmd}: {e}");
        }
        assert_eq!(s.active().unwrap().history.entries().len(), steps, "nothing changed");
        // Copy still copies, as in Photoshop.
        s.execute("edit.copy", json!({})).unwrap();
        // A catalogue query isn't an edit.
        assert!(s.execute("filter.filterGallery", json!({"list": true})).is_ok());
        // Shown again, Cut works.
        s.execute("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
        s.execute("edit.cut", json!({})).unwrap();
    }

    #[test]
    fn a_layer_in_a_hidden_group_is_hidden() {
        let (mut s, id) = session();
        s.execute("layer.groupLayers", json!({})).unwrap();
        let g = s.active().unwrap().active_layer.unwrap().0;
        assert_ne!(g, id, "the new group is active");
        s.execute("layer.setProps", json!({"layer": g, "visible": false})).unwrap();
        s.execute("layer.select", json!({"layer": id})).unwrap();
        assert!(s.execute("paint.stroke", json!({"points": [[10, 10]]})).unwrap_err().to_string().contains(HIDDEN));
    }

    #[test]
    fn alpha_channels_of_a_hidden_layer_still_paint() {
        let (mut s, id) = session();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        s.execute("channel.new", json!({})).unwrap();
        s.execute("paint.stroke", json!({"points": [[10, 10], [20, 20]], "target": {"channel": 0}})).unwrap();
    }
}
