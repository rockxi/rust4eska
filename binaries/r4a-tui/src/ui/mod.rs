pub mod dashboard;
pub mod not_implemented;

#[derive(Clone, Copy, PartialEq)]
pub enum Screen {
    Dashboard,
    Rbac,
    Manifests,
    Observability,
}

impl Screen {
    pub const ALL: &'static [Screen] = &[
        Screen::Dashboard,
        Screen::Rbac,
        Screen::Manifests,
        Screen::Observability,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Screen::Dashboard => "Dashboard",
            Screen::Rbac => "RBAC",
            Screen::Manifests => "Manifests",
            Screen::Observability => "Observability",
        }
    }

    pub fn next(&self) -> Screen {
        let idx = Screen::ALL.iter().position(|s| s == self).unwrap_or(0);
        Screen::ALL[(idx + 1) % Screen::ALL.len()]
    }

    pub fn prev(&self) -> Screen {
        let idx = Screen::ALL.iter().position(|s| s == self).unwrap_or(0);
        Screen::ALL[(idx + Screen::ALL.len() - 1) % Screen::ALL.len()]
    }
}
