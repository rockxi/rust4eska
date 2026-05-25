pub mod dashboard;
pub mod git;
pub mod manifests;
pub mod not_implemented;
pub mod rbac;
pub mod update;
pub mod vault;

#[derive(Clone, Copy, PartialEq)]
pub enum Screen {
    Dashboard,
    Manifests,
    Git,
    Vault,
    Rbac,
    Observability,
    Update,
}

impl Screen {
    pub const ALL: &'static [Screen] = &[
        Screen::Dashboard,
        Screen::Manifests,
        Screen::Git,
        Screen::Vault,
        Screen::Rbac,
        Screen::Observability,
        Screen::Update,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Screen::Dashboard => "Dashboard",
            Screen::Manifests => "Manifests",
            Screen::Git => "Git",
            Screen::Vault => "Vault",
            Screen::Rbac => "RBAC",
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
