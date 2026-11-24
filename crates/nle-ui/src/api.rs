use crate::types::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(catch, js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

async fn call_tauri<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let js_args = serde_wasm_bindgen::to_value(args).map_err(|e| e.to_string())?;
    let res = invoke(cmd, js_args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(res).map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TargetArg {
    target_format: NleFormat,
}

pub async fn pick_and_inspect_project(
    target_format: NleFormat,
) -> Result<Option<LoadedProjectPayload>, String> {
    call_tauri("pick_and_inspect_project", &TargetArg { target_format }).await
}

pub async fn load_demo_project(target_format: NleFormat) -> Result<LoadedProjectPayload, String> {
    call_tauri("load_demo_project", &TargetArg { target_format }).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReanalyzeArgs {
    selected_sequence_ids: Vec<String>,
    target_format: NleFormat,
}

pub async fn reanalyze_effects(
    selected_sequence_ids: Vec<String>,
    target_format: NleFormat,
) -> Result<EffectMappingPlan, String> {
    call_tauri(
        "reanalyze_effects",
        &ReanalyzeArgs {
            selected_sequence_ids,
            target_format,
        },
    )
    .await
}

#[derive(Serialize)]
struct ConvertArgs {
    options: ConversionOptions,
    decisions: UserEffectDecisions,
}

pub async fn pick_output_and_convert(
    options: ConversionOptions,
    decisions: UserEffectDecisions,
) -> Result<Option<ConversionReport>, String> {
    call_tauri("pick_output_and_convert", &ConvertArgs { options, decisions }).await
}

#[derive(Serialize)]
struct SavePresetArgs {
    preset: EffectMappingPreset,
}

pub async fn save_effect_preset(preset: EffectMappingPreset) -> Result<Option<String>, String> {
    call_tauri("save_effect_preset", &SavePresetArgs { preset }).await
}

pub async fn load_effect_preset() -> Result<Option<EffectMappingPreset>, String> {
    #[derive(Serialize)]
    struct Empty {}
    call_tauri("load_effect_preset", &Empty {}).await
}

#[derive(Serialize)]
struct RevealArgs {
    path: String,
}

pub async fn reveal_in_file_manager(path: String) -> Result<(), String> {
    call_tauri("reveal_in_file_manager", &RevealArgs { path }).await
}
