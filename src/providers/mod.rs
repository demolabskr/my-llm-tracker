pub mod claude;
pub mod openai;
pub mod opencode;

/// One usage window (e.g. 5h / 7d / 30d). `left` is the remaining percentage.
#[derive(Clone)]
pub struct Win {
    pub label: String,
    pub left: f32,
    pub reset_at: Option<i64>,
}

impl Win {
    pub fn from_used(label: impl Into<String>, used_pct: f64, reset_at: Option<i64>) -> Win {
        Win { label: label.into(), left: (100.0 - used_pct).clamp(0.0, 100.0) as f32, reset_at }
    }
}

pub type Fetched = Result<(String, Vec<Win>), String>;
