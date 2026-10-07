//! PathForge core: scene settings, CPU and GPU renderers, and exporters.
//! The GUI (`path_forge`) and the headless CLI (`pf`) are thin binaries over this library.

pub mod settings;
pub mod scene;
pub mod docs;
pub mod world;
pub mod export;
pub mod review;
pub mod mcp;
pub mod skill;
pub mod project;
pub mod runtime;
pub mod journey;
pub mod tiles;
pub mod renderer;
#[cfg(feature = "studio")]
pub mod studio;
#[cfg(feature = "studio")]
pub mod app;
#[cfg(feature = "studio")]
pub mod node_lab;
#[cfg(feature = "gpu")]
pub mod gif_export;
#[cfg(feature = "gpu")]
pub mod gpu_effects;
#[cfg(feature = "gpu")]
pub mod gpu_scene;
