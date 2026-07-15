pub mod dashboard;
pub mod git;
pub mod logs;
pub mod manifests;
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
    Logs,
    Update,
}

impl Screen {
    pub const ALL: &'static [Screen] = &[
        Screen::Dashboard,
        Screen::Manifests,
        Screen::Git,
        Screen::Vault,
        Screen::Rbac,
        Screen::Logs,
        Screen::Update,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Screen::Dashboard => "Dashboard",
            Screen::Manifests => "Manifests",
            Screen::Git => "Git",
            Screen::Vault => "Vault",
            Screen::Rbac => "RBAC",
            Screen::Logs => "Logs",
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
