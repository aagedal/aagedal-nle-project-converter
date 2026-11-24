pub mod catalog;
pub mod mapper;

pub use catalog::{builtin_effect_catalog, find_catalog_rule, EffectRule, MappingFidelity, TargetEffectOption};
pub use mapper::{
    analyze_project_effects, apply_effect_decisions, DetectedEffectGroup, EffectApplicationSummary,
    EffectMappingPlan, EffectMappingPreset, MappingAction, UserEffectDecisions,
};
