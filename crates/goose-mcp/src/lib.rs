use etcetera::AppStrategyArgs;
use once_cell::sync::Lazy;

pub static APP_STRATEGY: Lazy<AppStrategyArgs> = Lazy::new(|| AppStrategyArgs {
    top_level_domain: "Block".to_string(),
    author: "Block".to_string(),
    app_name: "goose".to_string(),
});

pub mod autovisualiser;
pub mod computercontroller;
pub mod developer;
mod memory;
mod tutorial;

pub use autovisualiser::AutoVisualiserRouter;
pub use computercontroller::ComputerControllerRouter;
pub use developer::rmcp_developer::DeveloperServer;
pub use memory::MemoryRouter;
pub use tutorial::TutorialRouter;
