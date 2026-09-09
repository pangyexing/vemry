//! The lab of a note's `js` block: the declarative experiment `vemry.lab.*` builds
//! while the block runs. The JS glue (bindings.rs) normalises the flexible call
//! shapes into this canonical JSON; this module is the typed contract the editor
//! renders from (mirrored in `src/lib/api.ts`), plus the caps that keep one block
//! from handing the UI a million cells.
//!
//! Nothing here executes anything: a block still runs only when a person clicks
//! Run, and re-runs only while that person moves the controls the block declared
//! (docs/SYNTAX.md, "Runnable code blocks"). The values the controls currently
//! hold travel back into the next run as `params` and never touch the note.

use serde::{Deserialize, Serialize};

/// Every control the block declared, with the value it ran with, and every view it
/// drew, in call order.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct LabSpec {
    pub controls: Vec<LabControl>,
    pub views: Vec<LabView>,
}

/// A control the block asked for. `value` is what this run used: the block's
/// default, or the value the user last set (handed back as `params`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LabControl {
    Slider { name: String, label: Option<String>, min: f64, max: f64, step: f64, value: f64 },
    Toggle { name: String, label: Option<String>, value: bool },
    Select { name: String, label: Option<String>, options: Vec<String>, value: String },
    /// An editable grid of numbers (a Q, K or V matrix, a small dataset).
    Matrix { name: String, label: Option<String>, rows: Vec<Vec<f64>>, min: Option<f64>, max: Option<f64>, step: Option<f64> },
    /// A row of buttons, each setting several of the other controls at once — the
    /// guided experiments of a card ("all keys equal", "hide the future"). `value`
    /// is the label last pressed, if any; the card writes the targets' values into
    /// `params` itself and re-runs.
    Presets { name: String, label: Option<String>, items: Vec<LabPreset>, value: Option<String> },
}

/// One preset: the button's label and the controls it sets, by name.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LabPreset {
    pub label: String,
    pub set: std::collections::BTreeMap<String, serde_json::Value>,
}

impl LabControl {
    pub fn name(&self) -> &str {
        match self {
            LabControl::Slider { name, .. }
            | LabControl::Toggle { name, .. }
            | LabControl::Select { name, .. }
            | LabControl::Matrix { name, .. }
            | LabControl::Presets { name, .. } => name,
        }
    }
}

/// A y-series of a line chart; `x` is per-series or absent (the view's shared `x`, else the index).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LineSeries {
    pub name: Option<String>,
    pub y: Vec<Option<f64>>,
    pub x: Option<Vec<Option<f64>>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BarSeries {
    pub name: Option<String>,
    pub values: Vec<Option<f64>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PointSeries {
    pub name: Option<String>,
    pub points: Vec<[f64; 2]>,
}

/// A table cell: a number (formatted by the UI) or text; an empty cell is `None`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Cell {
    Number(f64),
    Text(String),
}

/// The value a derivation step shows beside its text: a scalar, a vector or a grid.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum StepValue {
    Number(f64),
    Vector(Vec<Option<f64>>),
    Grid(Vec<Vec<Option<f64>>>),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StepItem {
    pub label: String,
    pub text: Option<String>,
    pub value: Option<StepValue>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Highlight {
    pub row: Option<usize>,
    pub col: Option<usize>,
}

/// The six views (docs/SYNTAX.md lists them; the cap is deliberate — a small API is
/// one a model writes correctly).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LabView {
    Heatmap {
        title: String,
        data: Vec<Vec<Option<f64>>>,
        rows: Option<Vec<String>>,
        cols: Option<Vec<String>>,
        min: Option<f64>,
        max: Option<f64>,
        digits: Option<u8>,
        /// Two poles around zero (weights that can be negative) rather than one hue.
        diverging: bool,
    },
    Line {
        title: String,
        series: Vec<LineSeries>,
        x: Option<Vec<Option<f64>>>,
        x_label: Option<String>,
        y_label: Option<String>,
        y_min: Option<f64>,
        y_max: Option<f64>,
    },
    Bars {
        title: String,
        labels: Vec<String>,
        series: Vec<BarSeries>,
        y_label: Option<String>,
        horizontal: bool,
    },
    Scatter {
        title: String,
        series: Vec<PointSeries>,
        x_label: Option<String>,
        y_label: Option<String>,
    },
    Table {
        title: String,
        columns: Vec<String>,
        rows: Vec<Vec<Option<Cell>>>,
        digits: Option<u8>,
        highlight: Option<Highlight>,
    },
    /// A derivation or an algorithm's trace, revealed one step at a time.
    Step { title: String, steps: Vec<StepItem> },
}

impl LabView {
    pub fn title(&self) -> &str {
        match self {
            LabView::Heatmap { title, .. }
            | LabView::Line { title, .. }
            | LabView::Bars { title, .. }
            | LabView::Scatter { title, .. }
            | LabView::Table { title, .. }
            | LabView::Step { title, .. } => title,
        }
    }
}

pub const MAX_CONTROLS: usize = 12;
pub const MAX_PRESETS: usize = 8;
pub const MAX_VIEWS: usize = 8;
pub const MAX_OPTIONS: usize = 32;
pub const MAX_MATRIX: usize = 16;
pub const MAX_HEATMAP: usize = 64;
pub const MAX_SERIES: usize = 8;
pub const MAX_POINTS: usize = 2048;
pub const MAX_BARS: usize = 64;
pub const MAX_TABLE_ROWS: usize = 200;
pub const MAX_TABLE_COLS: usize = 16;
pub const MAX_STEPS: usize = 64;
pub const MAX_TEXT: usize = 400;

fn cap(what: &str, n: usize, max: usize) -> Result<(), String> {
    if n > max {
        Err(format!("lab: {what} has {n}; the limit is {max}"))
    } else {
        Ok(())
    }
}

fn clip(s: &mut String) {
    if s.chars().count() > MAX_TEXT {
        *s = s.chars().take(MAX_TEXT).collect::<String>() + "…";
    }
}

fn clip_opt(s: &mut Option<String>) {
    if let Some(s) = s {
        clip(s);
    }
}

/// Parse what the glue handed back and enforce the caps. An over-cap block is an
/// error naming the cap (the author sees it under the fence), not a silent cut.
pub fn parse(json: &str) -> Result<LabSpec, String> {
    let mut spec: LabSpec = serde_json::from_str(json).map_err(|e| format!("lab: malformed spec: {e}"))?;
    cap("controls", spec.controls.len(), MAX_CONTROLS)?;
    cap("views", spec.views.len(), MAX_VIEWS)?;
    let declared: std::collections::HashSet<String> = spec.controls.iter().map(|c| c.name().to_string()).collect();
    let mut seen = std::collections::HashSet::new();
    for c in &mut spec.controls {
        if !seen.insert(c.name().to_string()) {
            return Err(format!("lab: the control `{}` is declared twice", c.name()));
        }
        match c {
            LabControl::Slider { name, label, min, max, step, value } => {
                clip(name);
                clip_opt(label);
                if !(min.is_finite() && max.is_finite() && step.is_finite() && *step > 0.0 && max > min) {
                    return Err(format!("lab: the slider `{name}` needs finite min < max and a positive step"));
                }
                *value = value.clamp(*min, *max);
            }
            LabControl::Toggle { name, label, .. } => {
                clip(name);
                clip_opt(label);
            }
            LabControl::Select { name, label, options, value } => {
                clip(name);
                clip_opt(label);
                cap(&format!("the select `{name}`"), options.len(), MAX_OPTIONS)?;
                if options.is_empty() {
                    return Err(format!("lab: the select `{name}` has no options"));
                }
                for o in options.iter_mut() {
                    clip(o);
                }
                if !options.contains(value) {
                    *value = options[0].clone();
                }
            }
            LabControl::Presets { name, label, items, value } => {
                clip(name);
                clip_opt(label);
                cap(&format!("the presets `{name}`"), items.len(), MAX_PRESETS)?;
                if items.is_empty() {
                    return Err(format!("lab: the presets `{name}` have no items"));
                }
                for it in items.iter_mut() {
                    clip(&mut it.label);
                    cap(&format!("the preset `{}`", it.label), it.set.len(), MAX_CONTROLS)?;
                    for (target, v) in &it.set {
                        // a preset names the controls it sets: one the block never declared is a mistake by name
                        if target == name || !declared.contains(target) {
                            return Err(format!("lab: the preset `{}` sets `{target}`, which is not a control of this block", it.label));
                        }
                        let grid_ok = v.as_array().is_some_and(|rows| rows.iter().all(|r| r.as_array().is_some_and(|r| r.iter().all(|x| x.as_f64().is_some_and(f64::is_finite)))));
                        if !(v.is_number() || v.is_boolean() || v.is_string() || grid_ok) {
                            return Err(format!("lab: the preset `{}` sets `{target}` to a value that is not a number, a boolean, a string or a grid", it.label));
                        }
                    }
                }
                if value.as_ref().is_some_and(|v| !items.iter().any(|it| &it.label == v)) {
                    *value = None;
                }
            }
            LabControl::Matrix { name, label, rows, .. } => {
                clip(name);
                clip_opt(label);
                cap(&format!("the matrix `{name}`"), rows.len(), MAX_MATRIX)?;
                let width = rows.first().map_or(0, |r| r.len());
                cap(&format!("the matrix `{name}`"), width, MAX_MATRIX)?;
                if width == 0 || rows.iter().any(|r| r.len() != width) {
                    return Err(format!("lab: the matrix `{name}` must be rectangular with at least one cell"));
                }
                if rows.iter().flatten().any(|v| !v.is_finite()) {
                    return Err(format!("lab: the matrix `{name}` holds a non-finite number"));
                }
            }
        }
    }
    for v in &mut spec.views {
        match v {
            LabView::Heatmap { title, data, rows, cols, .. } => {
                clip(title);
                let what = format!("the heatmap `{title}`");
                cap(&what, data.len(), MAX_HEATMAP)?;
                let width = data.iter().map(|r| r.len()).max().unwrap_or(0);
                cap(&what, width, MAX_HEATMAP)?;
                for r in rows.iter_mut().flatten().chain(cols.iter_mut().flatten()) {
                    clip(r);
                }
            }
            LabView::Line { title, series, x, x_label, y_label, .. } => {
                clip(title);
                clip_opt(x_label);
                clip_opt(y_label);
                let what = format!("the line chart `{title}`");
                cap(&what, series.len(), MAX_SERIES)?;
                if let Some(x) = x {
                    cap(&what, x.len(), MAX_POINTS)?;
                }
                for s in series.iter_mut() {
                    clip_opt(&mut s.name);
                    cap(&what, s.y.len(), MAX_POINTS)?;
                    if let Some(x) = &s.x {
                        cap(&what, x.len(), MAX_POINTS)?;
                    }
                }
            }
            LabView::Bars { title, labels, series, y_label, .. } => {
                clip(title);
                clip_opt(y_label);
                let what = format!("the bar chart `{title}`");
                cap(&what, labels.len(), MAX_BARS)?;
                cap(&what, series.len(), MAX_SERIES)?;
                for l in labels.iter_mut() {
                    clip(l);
                }
                for s in series.iter_mut() {
                    clip_opt(&mut s.name);
                    s.values.resize(labels.len(), None);
                }
            }
            LabView::Scatter { title, series, x_label, y_label } => {
                clip(title);
                clip_opt(x_label);
                clip_opt(y_label);
                let what = format!("the scatter `{title}`");
                cap(&what, series.len(), MAX_SERIES)?;
                for s in series.iter_mut() {
                    clip_opt(&mut s.name);
                    cap(&what, s.points.len(), MAX_POINTS)?;
                    if s.points.iter().flatten().any(|v| !v.is_finite()) {
                        return Err(format!("lab: the scatter `{title}` holds a non-finite point"));
                    }
                }
            }
            LabView::Table { title, columns, rows, .. } => {
                clip(title);
                let what = format!("the table `{title}`");
                cap(&what, columns.len(), MAX_TABLE_COLS)?;
                cap(&what, rows.len(), MAX_TABLE_ROWS)?;
                for c in columns.iter_mut() {
                    clip(c);
                }
                for r in rows.iter_mut() {
                    r.resize(columns.len(), None);
                    for cell in r.iter_mut() {
                        if let Some(Cell::Text(s)) = cell {
                            clip(s);
                        }
                    }
                }
            }
            LabView::Step { title, steps } => {
                clip(title);
                cap(&format!("the steps `{title}`"), steps.len(), MAX_STEPS)?;
                for s in steps.iter_mut() {
                    clip(&mut s.label);
                    clip_opt(&mut s.text);
                }
            }
        }
    }
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_canonical_shape_and_clamps() {
        let json = r#"{"controls":[
            {"kind":"slider","name":"d_k","label":null,"min":4,"max":512,"step":4,"value":9999},
            {"kind":"toggle","name":"scale","label":"scale","value":true},
            {"kind":"select","name":"head","label":null,"options":["a","b"],"value":"zzz"},
            {"kind":"matrix","name":"Q","label":null,"rows":[[1,0],[0,1]],"min":null,"max":null,"step":null},
            {"kind":"presets","name":"try","label":"try","items":[{"label":"no scale","set":{"scale":false,"d_k":64}}],"value":"gone"}
          ],"views":[
            {"kind":"heatmap","title":"attention","data":[[0.5,null],[0.25,0.75]],"rows":["a","b"],"cols":null,"min":null,"max":null,"digits":2,"diverging":false},
            {"kind":"line","title":"pe","series":[{"name":"sin","y":[0,1,0],"x":null}],"x":null,"x_label":"pos","y_label":null,"y_min":null,"y_max":null},
            {"kind":"bars","title":"bleu","labels":["base","big"],"series":[{"name":null,"values":[27.3]}],"y_label":null,"horizontal":false},
            {"kind":"scatter","title":"s","series":[{"name":null,"points":[[1,2]]}],"x_label":null,"y_label":null},
            {"kind":"table","title":"t","columns":["a","b"],"rows":[[1,"x"],[null]],"digits":null,"highlight":{"row":0,"col":null}},
            {"kind":"step","title":"softmax","steps":[{"label":"scores","text":null,"value":[[1,2]]},{"label":"exp","text":"e^x","value":3}]}
          ]}"#;
        let spec = parse(json).unwrap();
        assert_eq!(spec.controls.len(), 5);
        match &spec.controls[0] {
            LabControl::Slider { value, .. } => assert_eq!(*value, 512.0),
            _ => panic!(),
        }
        match &spec.controls[2] {
            LabControl::Select { value, .. } => assert_eq!(value, "a"),
            _ => panic!(),
        }
        // a short bar series is padded to the label count, a short table row to the column count
        match &spec.views[2] {
            LabView::Bars { series, .. } => assert_eq!(series[0].values, vec![Some(27.3), None]),
            _ => panic!(),
        }
        match &spec.views[4] {
            LabView::Table { rows, .. } => assert_eq!(rows[1].len(), 2),
            _ => panic!(),
        }
        match &spec.views[5] {
            LabView::Step { steps, .. } => assert_eq!(steps[1].value, Some(StepValue::Number(3.0))),
            _ => panic!(),
        }
        // a preset's value is the label last pressed; one that names no item is dropped
        match &spec.controls[4] {
            LabControl::Presets { items, value, .. } => {
                assert_eq!(items[0].label, "no scale");
                assert_eq!(items[0].set["d_k"], serde_json::json!(64));
                assert_eq!(*value, None);
            }
            _ => panic!(),
        }
        // the round trip is the wire format the editor reads
        let back: LabSpec = serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(back, spec);
    }

    #[test]
    fn a_preset_names_only_the_block_s_own_controls() {
        let json = r#"{"controls":[{"kind":"slider","name":"d_k","label":null,"min":4,"max":64,"step":4,"value":16},{"kind":"presets","name":"try","label":null,"items":[{"label":"x","set":{"d_k":64,"nothere":1}}],"value":null}],"views":[]}"#;
        let err = parse(json).unwrap_err();
        assert!(err.contains("`x` sets `nothere`"), "{err}");
        let json = r#"{"controls":[{"kind":"slider","name":"d_k","label":null,"min":4,"max":64,"step":4,"value":16},{"kind":"presets","name":"try","label":null,"items":[{"label":"x","set":{"d_k":{"a":1}}}],"value":null}],"views":[]}"#;
        let err = parse(json).unwrap_err();
        assert!(err.contains("not a number"), "{err}");
        let json = r#"{"controls":[{"kind":"presets","name":"try","label":null,"items":[],"value":null}],"views":[]}"#;
        assert!(parse(json).unwrap_err().contains("no items"));
    }

    #[test]
    fn caps_and_duplicates_are_errors_that_name_the_cap() {
        let big: Vec<Vec<f64>> = vec![vec![0.0; 65]; 2];
        let json = serde_json::json!({"controls": [], "views": [{"kind":"heatmap","title":"h","data": big,"rows":null,"cols":null,"min":null,"max":null,"digits":null,"diverging":false}]});
        let e = parse(&json.to_string()).unwrap_err();
        assert!(e.contains("the heatmap `h` has 65; the limit is 64"), "{e}");
        let json = r#"{"controls":[{"kind":"toggle","name":"a","label":null,"value":true},{"kind":"toggle","name":"a","label":null,"value":false}],"views":[]}"#;
        assert!(parse(json).unwrap_err().contains("declared twice"));
        let json = r#"{"controls":[{"kind":"slider","name":"s","label":null,"min":1,"max":1,"step":1,"value":1}],"views":[]}"#;
        assert!(parse(json).unwrap_err().contains("min < max"));
        assert!(parse("nonsense").unwrap_err().contains("malformed"));
    }
}
