//! Image › Adjustments › Replace Color… as Photoshop lays it out: Localized Color Clusters, the
//! three eyedroppers (sample, add, subtract) with the Color swatch, Fuzziness, Range, Hue,
//! Saturation and Lightness, and Preview. The eyedroppers are armed as soon as the dialog opens:
//! the image itself is the eyedropper (Shift adds to the sample, Alt subtracts), reusing Select ›
//! Color Range's picking (`color_range_ui::pick`) so both dialogs build identical samples.
//!
//! Everything lives in the dialog fields, so `ui.dialog.set` drives it. `__tool` is the selected
//! eyedropper; `points`, `subtractPoints` and `order` are the engine's own params.

use egui::Color32;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::widgets;

pub const COMMAND: &str = "image.adjustments.replaceColor";

/// Whether `f` is the Replace Color dialog.
pub(crate) fn owns(f: &Map<String, Value>) -> bool {
    f.get("__filter").and_then(Value::as_bool) == Some(true) && f.get("__command").and_then(Value::as_str) == Some(COMMAND)
}

/// Open (and Alt+Reset) state: the plain eyedropper armed, the thumbnail showing the Selection,
/// no samples yet.
pub(crate) fn defaults(f: &mut Map<String, Value>) {
    f.insert("__tool".into(), json!("sample"));
    f.insert("__view".into(), json!("selection"));
    f.insert("points".into(), json!([]));
    f.insert("subtractPoints".into(), json!([]));
    f.insert("order".into(), json!([]));
}

fn non_empty(f: &Map<String, Value>, k: &str) -> bool {
    f.get(k).and_then(Value::as_array).is_some_and(|a| !a.is_empty())
}

/// The `image.adjustments.replaceColor` params for the dialog's fields, for the live preview and
/// OK alike (both go through `filter_dialog::params_of`). The engine rejects Localized Color
/// Clusters without a sampled position, so `localized` and `range` are only sent once there is
/// one; empty sample lists are left out.
pub(crate) fn params(f: &Map<String, Value>) -> Value {
    let mut p: Map<String, Value> = f.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
    for k in ["points", "subtractPoints", "order"] {
        if !non_empty(f, k) {
            p.remove(k);
        }
    }
    if !non_empty(f, "points") {
        p.remove("localized");
        p.remove("range");
    }
    Value::Object(p)
}

/// Whether the top dialog is Replace Color: the canvas is its eyedropper from the moment it opens.
pub(crate) fn sampling_top(app: &PhotocraftApp) -> bool {
    app.ui.dialogs.last().is_some_and(|d| owns(&d.fields))
}

/// An eyedropper click at document point `at` (Shift adds, Alt subtracts). A plain click samples:
/// it replaces the samples and sets From `color` to the clicked pixel's merged colour. Returns
/// whether the click landed on the image.
pub(crate) fn pick_top(app: &mut PhotocraftApp, at: [f64; 2], mods: egui::Modifiers) -> bool {
    let Some(mut f) = app.ui.dialogs.last().map(|d| &d.fields).filter(|f| owns(f)).cloned() else { return false };
    if !pick_fields(app, &mut f, at, mods) {
        return false;
    }
    match app.ui.dialogs.last_mut() {
        Some(d) => {
            d.fields = f;
            true
        }
        None => false,
    }
}

/// [`pick_top`] on a copy of the dialog's fields: the dialog body holds its own copy while it is
/// drawn, so a click on the thumbnail has to land in that one.
fn pick_fields(app: &mut PhotocraftApp, f: &mut Map<String, Value>, at: [f64; 2], mods: egui::Modifiers) -> bool {
    let Some(doc) = app.session.active().map(|st| &st.doc) else { return false };
    if !at[0].is_finite() || !at[1].is_finite() || at[0] < 0.0 || at[1] < 0.0 || at[0] >= f64::from(doc.size.width) || at[1] >= f64::from(doc.size.height) {
        return false;
    }
    let tool = crate::color_range_ui::tool_of(f, mods).to_string();
    crate::color_range_ui::pick(app, f, at, mods, "order");
    // A plain sample sets From colour to the sampled pixel (its own colour keeps the Lab box
    // unchanged). Add and subtract only reshape the match: they never replace a From colour, which
    // may have been chosen in its Color Picker.
    let pts = f.get("points").and_then(Value::as_array).cloned().unwrap_or_default();
    let first = pts.first().and_then(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)));
    if let Some((x, y)) = first
        && tool == "sample"
        && let Some(rgb) = crate::canvas::composite_color(app, x, y)
    {
        f.insert("color".into(), json!(crate::color_picker_ui::hex(rgb)));
    }
    if let Some(c) = f.get("color").cloned() {
        f.insert("__sampledColor".into(), c);
    }
    true
}

/// The canvas under the dialog: the eyedropper cursor at `over` (+ / − while Shift / Alt are
/// held), and a sample at document point `at` for a press. Add and subtract take a point per
/// press; only the plain eyedropper keeps sampling while the pointer drags.
pub(crate) fn canvas(app: &mut PhotocraftApp, ctx: &egui::Context, over: egui::Pos2, at: Option<[f64; 2]>) {
    let Some(f) = app.ui.dialogs.last().map(|d| &d.fields).filter(|f| owns(f)).cloned() else { return };
    let mods = ctx.input(|i| i.modifiers);
    let tool = crate::color_range_ui::tool_of(&f, mods);
    crate::color_range_ui::eyedropper_cursor(app, ctx, over, crate::color_range_ui::tool_badge(tool));
    let (pressed, moved) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.delta() != egui::Vec2::ZERO));
    if let Some(at) = at
        && (pressed || (tool == "sample" && moved))
        && pick_top(app, at, mods)
    {
        ctx.request_repaint();
    }
}

fn num(f: &Map<String, Value>, k: &str, d: f32) -> f32 {
    f.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}

fn rgb_of(f: &Map<String, Value>, k: &str) -> Option<[f32; 3]> {
    f.get(k).and_then(Value::as_str).and_then(crate::color_picker_ui::parse_hex)
}

/// The From colour the Color swatch shows, if `color` is a valid colour.
fn from_swatch(f: &Map<String, Value>) -> Option<[f32; 3]> {
    rgb_of(f, "color")
}

/// Frame-start sync of the Result swatch (`__result`, a hex) with the From `color` and the Hue,
/// Saturation and Lightness sliders.
///
/// * A From colour changed outside the eyedropper (its Color Picker) no longer matches the samples
///   (`__sampledColor` is the hex `pick_top` last set), so the samples are dropped.
/// * A `__result` the Color Picker wrote (it differs from `__resultSynced`, the hex this function
///   last wrote) sets Hue, Saturation and Lightness through `replace_color_shift`. Hue follows the
///   slider's sign: red to blue is -120. The shift is whole numbers, clamped to the slider ranges,
///   so `__result` is then rewritten to what the sliders actually give.
/// * Otherwise `__result` is rewritten to what the sliders give for the current From colour.
///
/// `__result`, `__resultSynced` and `__sampledColor` start with `__`, so `params` never sends them.
pub(crate) fn sync_result(f: &mut Map<String, Value>) {
    let Some(from) = rgb_of(f, "color") else { return };
    let color = crate::color_picker_ui::hex(from);
    match f.get("__sampledColor").and_then(Value::as_str) {
        Some(sampled) if sampled != color && non_empty(f, "points") => {
            for k in ["points", "subtractPoints", "order"] {
                f.insert(k.into(), json!([]));
            }
        }
        _ => {}
    }
    f.insert("__sampledColor".into(), json!(color));

    let picked = rgb_of(f, "__result").filter(|_| f.get("__result") != f.get("__resultSynced"));
    if let Some(result) = picked {
        // `+ 0.0` turns -0.0 into 0.0.
        for (k, v) in ["hue", "saturation", "lightness"].into_iter().zip(photocraft_algo::tone::replace_color_shift(from, result)) {
            f.insert(k.into(), json!(f64::from(v) + 0.0));
        }
    }
    let result =
        crate::color_picker_ui::hex(photocraft_algo::tone::replace_color_result(from, num(f, "hue", 0.0), num(f, "saturation", 0.0), num(f, "lightness", 0.0)));
    f.insert("__result".into(), json!(result));
    f.insert("__resultSynced".into(), json!(result));
}

/// Longest sides of the Selection / Image thumbnail, in points.
const THUMB: egui::Vec2 = egui::vec2(200.0, 150.0);

/// Photoshop's Selection / Image preview: the mask the sampled colours select (white = changed),
/// or the image to pick from. Clicking it picks like a canvas click, Shift adds, Alt subtracts;
/// Ctrl (Cmd on macOS) shows the other view while held.
fn thumbnail(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    use crate::color_range_ui::{Slot, mask_thumbnail, radio, thumbnail_error};
    let t = Tokens::get(ui.ctx());
    let view = f.get("__view").and_then(Value::as_str).unwrap_or("selection").to_string();
    let swap = ui.input(|i| i.modifiers.command || (!cfg!(target_os = "macos") && i.modifiers.ctrl));
    let (frame, resp) = ui.allocate_exact_size(THUMB, egui::Sense::click_and_drag());
    ui.painter().rect_filled(frame, 0.0, t.canvas);
    let shown = mask_thumbnail(app, ui.ctx(), Slot::ReplaceColor, &params(f), (view == "image") ^ swap);
    let doc_size = app.session.active().map(|st| [f64::from(st.doc.size.width.max(1)), f64::from(st.doc.size.height.max(1))]);
    if let (Some((tex, [w, h])), Some([dw, dh])) = (shown, doc_size) {
        let scale = (THUMB.x / w.max(1) as f32).min(THUMB.y / h.max(1) as f32);
        let img = egui::Rect::from_center_size(frame.center(), egui::vec2(w as f32 * scale, h as f32 * scale));
        ui.painter().image(tex, img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        // Response::clicked may take an accessibility input lock: ask before reading the pointer.
        let clicked = resp.clicked();
        let held = resp.is_pointer_button_down_on();
        let mods = ui.input(|i| i.modifiers);
        let sampling = if crate::color_range_ui::tool_of(f, mods) == "sample" {
            ui.input(|i| (clicked && i.pointer.primary_pressed()) || (held && (i.pointer.primary_pressed() || i.pointer.delta() != egui::Vec2::ZERO)))
        } else {
            clicked
        };
        if let Some(p) = resp.interact_pointer_pos().filter(|p| sampling && img.contains(*p)) {
            let at = [
                (f64::from((p.x - img.left()) / img.width()) * dw).floor().clamp(0.0, dw - 1.0),
                (f64::from((p.y - img.top()) / img.height()) * dh).floor().clamp(0.0, dh - 1.0),
            ];
            pick_fields(app, f, at, mods);
        }
    }
    if let Some(e) = thumbnail_error(app, Slot::ReplaceColor) {
        let msg = format!("{}\n{e}", tl!("Preview unavailable:"));
        let g = ui.painter().layout(msg, egui::FontId::proportional(11.0), t.text_faint, frame.width() - 16.0);
        ui.painter().rect_filled(frame, 0.0, t.canvas);
        ui.painter().galley(frame.center() - g.size() / 2.0, g, t.text_faint);
    }
    ui.painter().rect_stroke(frame, 0.0, egui::Stroke::new(1.0, t.field_border), egui::StrokeKind::Outside);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, ui.is_enabled(), tl!("Replace Color preview")));
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if radio(ui, view == "selection", tl!("Selection")).clicked() {
            f.insert("__view".into(), json!("selection"));
        }
        ui.add_space(8.0);
        if radio(ui, view == "image", tl!("Image")).clicked() {
            f.insert("__view".into(), json!("image"));
        }
    });
}

/// Dialog body, in Photoshop's order.
pub(crate) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    sync_result(f);
    let t = Tokens::get(ui.ctx());
    let localized = f.get("localized").and_then(Value::as_bool).unwrap_or(false);
    let mut on = localized;
    if widgets::checkbox(ui, &mut on, tl!("Localized Color Clusters")).changed() {
        f.insert("localized".into(), json!(on));
    }
    ui.add_space(4.0);
    let tool = f.get("__tool").and_then(Value::as_str).unwrap_or("sample").to_string();
    let name = crate::filter_dialog::label("color");
    ui.horizontal(|ui| {
        for (id, badge, label) in [("sample", "", tl!("Eyedropper Tool")), ("add", "+", tl!("Add to Sample")), ("subtract", "−", tl!("Subtract from Sample"))]
        {
            if crate::color_range_ui::eyedropper(ui, badge, tool == id, label).clicked() {
                f.insert("__tool".into(), json!(id));
            }
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new(&name).color(t.text_dim));
        // No swatch without a valid From colour: nothing stands in for it.
        if let Some(rgb) = from_swatch(f) {
            let b = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
            if widgets::color_swatch_button(ui, Color32::from_rgb(b[0], b[1], b[2]), &name).clicked() {
                crate::color_picker_ui::request_field(f, "color", &crate::color_picker_ui::title_for(&name));
            }
            ui.label(egui::RichText::new(crate::color_picker_ui::hex(rgb)).color(t.text_dim).monospace());
        }
    });
    ui.add_space(4.0);
    let slider = |ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, range: std::ops::RangeInclusive<f32>, default: f32| {
        let mut v = num(f, key, default);
        let g = widgets::RowGestures { reset: Some(default), wheel_step: None };
        widgets::slider_row_with(ui, &crate::filter_dialog::label(key), &mut v, range, "", None, g);
        f.insert(key.into(), json!(v.round()));
    };
    slider(ui, f, "fuzziness", 0.0..=200.0, 40.0);
    // Range only applies with Localized Color Clusters on, as in Photoshop.
    ui.add_enabled_ui(localized, |ui| slider(ui, f, "range", 0.0..=100.0, 100.0));
    ui.add_space(6.0);
    thumbnail(app, ui, f);
    ui.add_space(6.0);
    slider(ui, f, "hue", -180.0..=180.0, 0.0);
    slider(ui, f, "saturation", -100.0..=100.0, 0.0);
    slider(ui, f, "lightness", -100.0..=100.0, 0.0);
    // No Result row without a valid `__result`: nothing is drawn in its place.
    if let Some(rgb) = rgb_of(f, "__result") {
        ui.horizontal(|ui| {
            let name = tl!("Result");
            ui.label(egui::RichText::new(name).color(t.text_dim));
            let b = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
            if widgets::color_swatch_button(ui, Color32::from_rgb(b[0], b[1], b[2]), name).clicked() {
                crate::color_picker_ui::request_field(f, "__result", &crate::color_picker_ui::title_for(name));
            }
            ui.label(egui::RichText::new(crate::color_picker_ui::hex(rgb)).color(t.text_dim).monospace());
        });
    }
    if let Some(mut preview) = f.get("__preview").and_then(Value::as_bool) {
        ui.add_space(4.0);
        widgets::checkbox(ui, &mut preview, tl!("Preview"));
        f.insert("__preview".into(), json!(preview));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
    use photocraft_geom::Rect as GRect;

    /// 40 × 30: red block (0..20, 0..15), blue block (20..40, 0..15), black and white below.
    fn app_with_doc() -> PhotocraftApp {
        let mut doc = Document::with_background("rc", Size::new(40, 30), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let bg = doc.layers[0].surface_mut().unwrap();
        bg.fill_rect(GRect::new(0, 0, 20, 15), &[1.0, 0.0, 0.0, 1.0]);
        bg.fill_rect(GRect::new(20, 0, 40, 15), &[0.0, 0.0, 1.0, 1.0]);
        bg.fill_rect(GRect::new(0, 15, 20, 30), &[0.0, 0.0, 0.0, 1.0]);
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);
        PhotocraftApp::new(s, crate::Services::default())
    }

    fn fields(app: &PhotocraftApp) -> Map<String, Value> {
        app.ui.dialogs.last().unwrap().fields.clone()
    }

    #[test]
    fn opens_with_sample_eyedropper_armed() {
        let mut app = app_with_doc();
        crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        assert!(sampling_top(&app));
        assert_eq!(app.ui.dialogs.last().unwrap().fields["__tool"], json!("sample"));
    }

    #[test]
    fn plain_click_samples_and_sets_colour() {
        let mut app = app_with_doc();
        crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        assert!(pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE));
        let f = &app.ui.dialogs.last().unwrap().fields;
        assert_eq!(f["points"], json!([[5.0, 5.0]]));
        assert_eq!(f["color"], json!("#ff0000"));
        assert_eq!(f["order"], json!(["+"]));
    }

    #[test]
    fn shift_adds_and_alt_subtracts_in_click_order() {
        let mut app = app_with_doc();
        crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        pick_top(&mut app, [25.0, 5.0], egui::Modifiers::SHIFT);
        pick_top(&mut app, [25.0, 5.0], egui::Modifiers::ALT);
        let f = &app.ui.dialogs.last().unwrap().fields;
        assert_eq!(f["points"].as_array().unwrap().len(), 2);
        assert_eq!(f["subtractPoints"].as_array().unwrap().len(), 1);
        assert_eq!(f["order"], json!(["+", "+", "-"]));
        assert_eq!(f["color"], json!("#ff0000"), "From colour stays the first sample");
    }

    #[test]
    fn ok_runs_one_step_with_samples() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        app.ui.dialog_mut(id).unwrap().fields.insert("hue".into(), json!(120));
        let before = app.session.active().unwrap().history.past_len();
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
    }

    #[test]
    fn cancel_disarms_canvas_eyedropper() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        crate::dialogs::cancel(&mut app, id).unwrap();
        assert!(!sampling_top(&app));
        assert!(!pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE));
    }

    #[test]
    fn off_image_click_changes_nothing() {
        let mut app = app_with_doc();
        crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        let before = fields(&app);
        assert!(!pick_top(&mut app, [-1.0, 5.0], egui::Modifiers::NONE));
        assert!(!pick_top(&mut app, [5.0, 99.0], egui::Modifiers::NONE));
        assert_eq!(fields(&app), before);
    }

    /// R1: the engine rejects `localized` without `points`, so the dialog never sends it that way
    /// (neither to the live preview nor to OK, which both build their params with `params_of`).
    #[test]
    fn localized_is_only_sent_with_samples() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        {
            let f = &mut app.ui.dialog_mut(id).unwrap().fields;
            f.insert("localized".into(), json!(true));
            f.insert("range".into(), json!(50));
        }
        let p = crate::filter_dialog::params_of(&fields(&app));
        for k in ["localized", "range", "points", "subtractPoints", "order"] {
            assert!(p.get(k).is_none(), "{k} must not be sent without samples: {p}");
        }
        // OK with Localized ticked and no samples runs.
        crate::dialogs::confirm(&mut app, id).unwrap();

        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        app.ui.dialog_mut(id).unwrap().fields.insert("localized".into(), json!(true));
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        let p = crate::filter_dialog::params_of(&fields(&app));
        assert_eq!(p["localized"], json!(true));
        assert_eq!(p["points"], json!([[5.0, 5.0]]));
        assert_eq!(p["order"], json!(["+"]));
        assert!(p.get("subtractPoints").is_none());
        assert!(p.as_object().unwrap().keys().all(|k| !k.starts_with("__")));
    }

    #[test]
    fn thumbnail_mask_whites_out_sampled_red_only() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        let p = params(&app.ui.dialog_mut(id).unwrap().fields);
        let mask = crate::color_range_ui::mask_values(&app, crate::color_range_ui::Slot::ReplaceColor, &p).unwrap();
        let (w, _h) = mask.1;
        assert!(mask.0[5 * w + 5] > 0.9);
        assert!(mask.0[5 * w + 25] < 0.1);
    }

    /// OK changes only the active layer, so the thumbnail judges that layer's pixels: red under a
    /// blue layer above is still changed, and shown as changed.
    #[test]
    fn thumbnail_judges_the_active_layer_ok_changes() {
        use photocraft_doc::Layer;
        let mut doc = Document::with_background("two layers", Size::new(40, 30), ColorMode::Rgb, SampleType::U8, Color::rgb(1.0, 0.0, 0.0));
        let mut top = Layer::raster("blue", doc.pixel_format());
        top.surface_mut().unwrap().fill_rect(GRect::new(0, 0, 20, 30), &[0.0, 0.0, 1.0, 1.0]);
        doc.layers.push(top);
        let bottom = doc.layers[0].id;
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);
        s.select_layer(bottom).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        {
            let f = &mut app.ui.dialog_mut(id).unwrap().fields;
            f.insert("color".into(), json!("#ff0000"));
            f.insert("fuzziness".into(), json!(10));
            f.insert("hue".into(), json!(120));
        }
        let (mask, (w, _)) = crate::color_range_ui::mask_values(&app, crate::color_range_ui::Slot::ReplaceColor, &params(&fields(&app))).unwrap();
        crate::dialogs::confirm(&mut app, id).unwrap();
        let layer = app.session.active().unwrap().doc.layer(bottom).unwrap().surface().unwrap().clone();
        for x in [5, 30] {
            let changed = layer.rgba(x, 10)[1] > 0.5;
            assert!(changed, "OK recolours the active layer's red at x = {x}");
            assert!(mask[10 * w + x as usize] > 0.9, "the thumbnail shows x = {x} as changed: {}", mask[10 * w + x as usize]);
        }
    }

    /// Both dialogs share one cache type but not one cache: drawing one must not evict the other.
    #[test]
    fn thumbnail_caches_are_separate_per_dialog() {
        use crate::color_range_ui::{Slot, mask_thumbnail};
        let mut app = app_with_doc();
        let ctx = egui::Context::default();
        let p = json!({"select": "sampledColors", "points": [[5, 5]], "fuzziness": 20});
        let q = json!({"select": "sampledColors", "points": [[25, 5]], "fuzziness": 20});
        let a = mask_thumbnail(&mut app, &ctx, Slot::ColorRange, &p, false).unwrap().0;
        let b = mask_thumbnail(&mut app, &ctx, Slot::ReplaceColor, &q, false).unwrap().0;
        assert_ne!(a, b);
        assert_eq!(mask_thumbnail(&mut app, &ctx, Slot::ColorRange, &p, false).unwrap().0, a, "Color Range's texture survives");
        let image = mask_thumbnail(&mut app, &ctx, Slot::ReplaceColor, &q, true).unwrap();
        assert_ne!(image.0, b);
        assert_eq!(image.1, [40, 30]);
    }

    /// The reduced live preview takes its samples from the original document, as OK does: a
    /// sample between thumbnail pixels must not read the averaged neighbour.
    #[test]
    fn reduced_preview_matches_the_original_eyedropper_sample() {
        let mut doc = Document::with_background("fine detail", Size::new(400, 4), ColorMode::Rgb, SampleType::U8, Color::rgb(1.0, 0.0, 0.0));
        let surface = doc.layers[0].surface_mut().unwrap();
        surface.fill_rect(GRect::new(200, 0, 400, 4), &[0.0, 0.0, 1.0, 1.0]);
        surface.fill_rect(GRect::new(101, 0, 102, 4), &[0.0, 0.0, 1.0, 1.0]);
        let mut session = photocraft_engine::Session::new();
        session.add_document(doc, None);
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [101.0, 1.0], egui::Modifiers::NONE);
        {
            let f = &mut app.ui.dialog_mut(id).unwrap().fields;
            f.insert("fuzziness".into(), json!(0));
            f.insert("hue".into(), json!(120));
        }
        let params = crate::filter_dialog::params_of(&fields(&app));
        let st = app.session.active().unwrap();
        let (original, active) = (st.doc.clone(), st.active_layer);
        let shown = crate::filter_dialog::preview_document(&original, active, COMMAND, &params, 2).unwrap();
        let at = |d: &Document, x: i32| d.layers[0].surface().unwrap().read_region(GRect::new(x, 0, x + 1, 1));
        let changed = |d: &Document, x: i32, from: &Document, fx: i32| at(d, x) != at(from, fx);
        crate::dialogs::confirm(&mut app, id).unwrap();
        let ok = &app.session.active().unwrap().doc;
        assert!(changed(ok, 300, &original, 300), "OK recolours the blue half");
        assert!(!changed(ok, 0, &original, 0), "OK leaves the red background");
        let reduced = crate::proxy::proxy_document(&original, 2);
        assert!(changed(&shown, 150, &reduced, 150), "preview must recolour the same blue pixels as OK");
        assert!(!changed(&shown, 0, &reduced, 0), "preview must leave the red background alone");
    }

    /// Localized Color Clusters on a reduced preview of a layer whose content starts away from the
    /// origin: the clusters are centred where the sample was picked, as OK centres them.
    #[test]
    fn reduced_localized_preview_matches_ok_on_an_offset_layer() {
        use photocraft_doc::Layer;
        let mut doc = Document::with_background("offset", Size::new(400, 40), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut layer = Layer::raster("red", doc.pixel_format());
        let surface = layer.surface_mut().unwrap();
        surface.fill_rect(GRect::new(200, 0, 300, 40), &[1.0, 0.0, 0.0, 1.0]);
        surface.fill_rect(GRect::new(360, 0, 400, 40), &[1.0, 0.0, 0.0, 1.0]);
        let id = layer.id;
        doc.layers.push(layer);
        let p = json!({"color": "#ff0000", "points": [[210, 20]], "localized": true, "range": 25, "fuzziness": 20, "hue": 120});
        let shown = crate::filter_dialog::preview_document(&doc, Some(id), COMMAND, &p, 2).unwrap();
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);
        s.select_layer(id).unwrap();
        s.execute(COMMAND, p).unwrap();
        let ok = crate::proxy::proxy_document(&s.active().unwrap().doc, 2);
        let row = |d: &Document| d.layer(id).unwrap().surface().unwrap().read_region(GRect::new(100, 10, 200, 11));
        let (ok, shown) = (row(&ok), row(&shown));
        let worst = ok.iter().zip(&shown).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(worst < 0.03, "preview differs from OK by {worst} across the matched cluster");
        assert!(ok[4 * 10 + 1] > 0.8, "OK recolours red near the sample to green");
    }

    #[test]
    fn result_swatch_tracks_sliders() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        let f = &mut app.ui.dialog_mut(id).unwrap().fields;
        f.insert("hue".into(), json!(120));
        sync_result(f);
        assert_eq!(f["__result"], json!("#00ff00"));
    }

    /// Hue convention: the shift from red to blue is -120 (replace_color_shift's sign).
    #[test]
    fn picking_a_result_sets_hue_saturation_lightness() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        let f = &mut app.ui.dialog_mut(id).unwrap().fields;
        sync_result(f);
        assert_eq!(f["__result"], json!("#ff0000"));
        f.insert("__result".into(), json!("#0000ff"));
        sync_result(f);
        assert_eq!(f["hue"].as_f64(), Some(-120.0));
        assert_eq!(f["saturation"].as_f64(), Some(0.0));
        assert_eq!(f["lightness"].as_f64(), Some(0.0));
        assert_eq!(f["__result"], json!("#0000ff"));
    }

    /// A new sample moves From colour, which must not be read as the user picking a new Result.
    #[test]
    fn resampling_keeps_the_shift_and_updates_the_result() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        app.ui.dialog_mut(id).unwrap().fields.insert("hue".into(), json!(120));
        sync_result(&mut app.ui.dialog_mut(id).unwrap().fields);
        pick_top(&mut app, [25.0, 5.0], egui::Modifiers::NONE);
        let f = &mut app.ui.dialog_mut(id).unwrap().fields;
        sync_result(f);
        assert_eq!(f["hue"].as_f64(), Some(120.0));
        assert_eq!(f["points"], json!([[25.0, 5.0]]), "an eyedropper sample is not cleared");
        assert_ne!(f["__result"], json!("#00ff00"));
    }

    /// Add to Sample grows the match; it never replaces a From colour chosen in its Color Picker.
    #[test]
    fn shift_add_keeps_a_from_colour_picked_in_the_color_picker() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        {
            let f = &mut app.ui.dialog_mut(id).unwrap().fields;
            f.insert("color".into(), json!("#00ff00"));
            sync_result(f);
        }
        assert!(pick_top(&mut app, [5.0, 5.0], egui::Modifiers::SHIFT));
        let f = fields(&app);
        assert_eq!(f["points"], json!([[5.0, 5.0]]), "the added sample is kept");
        assert_eq!(f["color"], json!("#00ff00"), "Shift-click must not overwrite the picked From colour");
    }

    #[test]
    fn picking_from_colour_clears_samples() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        let f = &mut app.ui.dialog_mut(id).unwrap().fields;
        f.insert("color".into(), json!("#00ff00"));
        sync_result(f);
        assert_eq!(f["points"], json!([]));
        assert_eq!(f["subtractPoints"], json!([]));
        assert_eq!(f["order"], json!([]));
    }

    /// `__result` and the sync bookkeeping never reach the engine, and OK still runs.
    #[test]
    fn result_never_reaches_the_engine() {
        let mut app = app_with_doc();
        let id = crate::filter_dialog::open(&mut app, COMMAND).unwrap();
        pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE);
        let f = &mut app.ui.dialog_mut(id).unwrap().fields;
        sync_result(f);
        assert!(f.contains_key("__result"));
        let p = crate::filter_dialog::params_of(f);
        assert!(p.as_object().unwrap().keys().all(|k| !k.starts_with("__")), "{p}");
        crate::dialogs::confirm(&mut app, id).unwrap();
    }

    /// The control protocol's pointer samples like a canvas click and never reaches the tool.
    #[test]
    fn control_pointer_samples_with_modifiers_and_never_paints_under_the_dialog() {
        use egui_kittest::Harness;
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                crate::menus::menu_bar(app, ui);
                crate::dialogs::show(app, ui.ctx());
            },
            app_with_doc(),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(2);
        crate::filter_dialog::open(h.state_mut(), COMMAND).unwrap();
        let doc = h.state().session.active().unwrap();
        let (before, history) = (doc.revision, doc.history.past_len());
        let ctx = h.ctx.clone();
        for (events, mods) in [
            (json!([{"kind":"down","x":2,"y":2},{"kind":"move","x":3,"y":2},{"kind":"up","x":3,"y":2}]), json!({})),
            (json!([{"kind":"down","x":30,"y":2},{"kind":"up","x":30,"y":2}]), json!({"shift":true})),
            (json!([{"kind":"down","x":31,"y":3},{"kind":"up","x":31,"y":3}]), json!({"alt":true})),
        ] {
            let mut p = json!({"events": events});
            p["modifiers"] = mods;
            let (req, _) = crate::control::ControlRequest::new("ui.pointer", p);
            let _ = crate::control::handle(h.state_mut(), &ctx, &req);
        }
        let f = &h.state().ui.dialogs.last().unwrap().fields;
        assert_eq!(f["points"], json!([[3.0, 2.0], [30.0, 2.0]]));
        assert_eq!(f["subtractPoints"], json!([[31.0, 3.0]]));
        assert_eq!(f["order"], json!(["+", "+", "-"]));
        assert_eq!(f["color"], json!("#ff0000"), "From colour is the first sample");
        let doc = h.state().session.active().unwrap();
        assert_eq!((doc.revision, doc.history.past_len()), (before, history), "nothing painted");
        assert!(h.state().drag.is_none());
    }

    /// A click on the thumbnail picks like a canvas click, and the radios switch its view.
    #[test]
    fn thumbnail_click_picks_and_radios_switch_the_view() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut h = Harness::builder()
            .with_size(egui::vec2(900.0, 900.0))
            .build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app_with_doc());
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        crate::filter_dialog::open(h.state_mut(), COMMAND).unwrap();
        h.run_steps(4);
        assert_eq!(fields(h.state())["__view"], json!("selection"), "Selection is the default view");
        // 40 x 30 doc in the 200 x 150 thumbnail: 5 pt per document pixel.
        let at = h.get_by_label("Replace Color preview").rect().left_top() + egui::vec2(25.0 * 5.0 + 2.5, 5.0 * 5.0 + 2.5);
        h.hover_at(at);
        h.run_steps(1);
        h.drag_at(at);
        h.run_steps(1);
        h.drop_at(at);
        h.run_steps(2);
        let f = fields(h.state());
        assert_eq!(f["points"], json!([[25.0, 5.0]]));
        assert_eq!(f["color"], json!("#0000ff"));
        h.get_by_label("Image").click();
        h.run_steps(2);
        assert_eq!(fields(h.state())["__view"], json!("image"));
        h.get_by_label("Selection").click();
        h.run_steps(2);
        assert_eq!(fields(h.state())["__view"], json!("selection"));
        assert!(crate::filter_dialog::params_of(&fields(h.state())).get("__view").is_none(), "view state is not an engine param");
    }

    #[test]
    fn from_swatch_is_not_drawn_without_a_valid_colour() {
        assert!(from_swatch(&Map::new()).is_none());
        let mut f = Map::new();
        f.insert("color".into(), json!("nope"));
        assert!(from_swatch(&f).is_none(), "no black stand-in for an invalid From colour");
        f.insert("color".into(), json!("#ff0000"));
        assert_eq!(from_swatch(&f), Some([1.0, 0.0, 0.0]));
    }

    #[test]
    fn result_row_is_not_drawn_without_a_valid_result() {
        assert!(rgb_of(&Map::new(), "__result").is_none());
        let mut f = Map::new();
        f.insert("__result".into(), json!("nope"));
        assert!(rgb_of(&f, "__result").is_none());
    }
}
