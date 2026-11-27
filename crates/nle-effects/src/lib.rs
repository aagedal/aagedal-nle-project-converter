pub mod catalog;
pub mod mapper;
pub mod plugins;

pub use catalog::{builtin_effect_catalog, find_catalog_rule, EffectRule, MappingFidelity, TargetEffectOption};
pub use mapper::{
    analyze_project_effects, analyze_project_effects_with_plugins, apply_effect_decisions,
    DetectedEffectGroup, EffectApplicationSummary, EffectMappingPlan, EffectMappingPreset,
    MappingAction, UserEffectDecisions,
};
pub use plugins::{
    normalize_plugin_name, CrossNlePluginSummary, InstalledPlugin, InstalledPluginRegistry,
    PluginFormat,
};
