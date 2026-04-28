pub mod dashboard;
pub mod git;
pub mod not_implemented;
pub mod update;

#[derive(Clone, Copy, PartialEq)]
pub enum Screen {
    Dashboard,
    Git,
    Rbac,
    Manifests,
    Observability,
    Update,
}

impl Screen {
    pub const ALL: &'static [Screen] = &[
        Screen::Dashboard,
        Screen::Git,
        Screen::Rbac,
        Screen::Manifests,
        Screen::Observability,
        Screen::Update,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Screen::Dashboard => "Dashboard",
            Screen::Git => "Git",
            Screen::Rbac => "RBAC",
            Screen::Manifests => "Manifests",
            Screen::Observability => "Observability",
            Screen::Update => "Update",
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
