use ostool::{
    Tool, ToolConfig,
    build::{
        CargoSelector, apply_cargo_selector,
        config::{BuildConfig, BuildSystem, Cargo},
    },
};

fn main() {
    let mut config = BuildConfig {
        system: BuildSystem::Cargo(Cargo::default()),
    };
    let selector = CargoSelector::new(Some("kernel".to_owned()), None);
    apply_cargo_selector(&mut config, &selector).unwrap();

    let mut tool = Tool::new(ToolConfig::default()).unwrap();
    tool.activate_build_config(&mut config, &selector).unwrap();
}
