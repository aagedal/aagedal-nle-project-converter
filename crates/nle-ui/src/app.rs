use crate::api;
use crate::types::*;
use leptos::prelude::*;
use std::collections::{BTreeMap, HashSet};
use wasm_bindgen_futures::spawn_local;

#[component]
pub fn App() -> impl IntoView {
    let (loaded, set_loaded) = signal(Option::<LoadedProjectPayload>::None);
    let (target_format, set_target_format) = signal(NleFormat::DaVinciResolve);
    let (selected_seq_ids, set_selected_seq_ids) = signal(HashSet::<String>::new());
    let (decisions, set_decisions) = signal(BTreeMap::<String, MappingAction>::new());

    // Conversion options
    let (include_bins, set_include_bins) = signal(true);
    let (include_markers, set_include_markers) = signal(true);
    let (include_clip_colors, set_include_clip_colors) = signal(true);
    let (include_lumetri, set_include_lumetri) = signal(true);
    let (inject_fx_markers, set_inject_fx_markers) = signal(true);
    let (relink_from, set_relink_from) = signal(String::new());
    let (relink_to, set_relink_to) = signal(String::new());

    // Status & Report
    let (busy, set_busy) = signal(false);
    let (error_msg, set_error_msg) = signal(Option::<String>::None);
    let (status_toast, set_status_toast) = signal(Option::<String>::None);
    let (report, set_report) = signal(Option::<ConversionReport>::None);

    let apply_loaded_payload = move |payload: LoadedProjectPayload| {
        let seq_set: HashSet<String> = payload
            .inspection
            .sequences
            .iter()
            .map(|s| s.id.clone())
            .collect();
        let mut default_actions = BTreeMap::new();
        for fx in &payload.effect_plan.effects {
            default_actions.insert(fx.key.clone(), fx.default_action.clone());
        }
        set_target_format.set(payload.effect_plan.target_format);
        set_selected_seq_ids.set(seq_set);
        set_decisions.set(default_actions);
        set_report.set(None);
        set_error_msg.set(None);
        set_loaded.set(Some(payload));
    };

    let on_open_file = move |_| {
        set_busy.set(true);
        set_error_msg.set(None);
        let fmt = target_format.get_untracked();
        spawn_local(async move {
            match api::pick_and_inspect_project(fmt).await {
                Ok(Some(payload)) => apply_loaded_payload(payload),
                Ok(None) => {}
                Err(e) => set_error_msg.set(Some(e)),
            }
            set_busy.set(false);
        });
    };

    let on_load_demo = move |_| {
        set_busy.set(true);
        set_error_msg.set(None);
        let fmt = target_format.get_untracked();
        spawn_local(async move {
            match api::load_demo_project(fmt).await {
                Ok(payload) => apply_loaded_payload(payload),
                Err(e) => set_error_msg.set(Some(e)),
            }
            set_busy.set(false);
        });
    };

    let trigger_reanalyze = move |new_seqs: Vec<String>, new_fmt: NleFormat| {
        spawn_local(async move {
            match api::reanalyze_effects(new_seqs, new_fmt).await {
                Ok(new_plan) => {
                    let mut next_actions = BTreeMap::new();
                    for fx in &new_plan.effects {
                        next_actions.insert(fx.key.clone(), fx.default_action.clone());
                    }
                    set_decisions.set(next_actions);
                    set_loaded.update(|opt| {
                        if let Some(p) = opt {
                            p.effect_plan = new_plan;
                        }
                    });
                }
                Err(e) => set_error_msg.set(Some(e)),
            }
        });
    };

    let on_change_target = move |ev: leptos::ev::Event| {
        let val = event_target_value(&ev);
        let new_fmt = NleFormat::from_str(&val);
        set_target_format.set(new_fmt);
        let seqs: Vec<String> = selected_seq_ids.get_untracked().into_iter().collect();
        trigger_reanalyze(seqs, new_fmt);
    };

    let on_convert = move |_| {
        let Some(current) = loaded.get_untracked() else {
            return;
        };
        set_busy.set(true);
        set_error_msg.set(None);
        set_status_toast.set(None);

        let mut relink_rules = Vec::new();
        let rf = relink_from.get_untracked();
        let rt = relink_to.get_untracked();
        if !rf.trim().is_empty() {
            relink_rules.push(PathRelinkRule {
                from_prefix: rf,
                to_prefix: rt,
            });
        }

        let options = ConversionOptions {
            target_format: target_format.get_untracked(),
            project_name: Some(current.inspection.name.clone()),
            selected_sequence_ids: selected_seq_ids.get_untracked().into_iter().collect(),
            include_bins: include_bins.get_untracked(),
            include_markers: include_markers.get_untracked(),
            include_clip_colors: include_clip_colors.get_untracked(),
            include_lumetri_grades: include_lumetri.get_untracked(),
            inject_effect_review_markers: inject_fx_markers.get_untracked(),
            relink_rules,
        };
        let user_decisions = UserEffectDecisions {
            actions: decisions.get_untracked(),
        };

        spawn_local(async move {
            match api::pick_output_and_convert(options, user_decisions).await {
                Ok(Some(rep)) => {
                    set_report.set(Some(rep));
                }
                Ok(None) => {}
                Err(e) => set_error_msg.set(Some(e)),
            }
            set_busy.set(false);
        });
    };

    let on_save_preset = move |_| {
        let Some(current) = loaded.get_untracked() else {
            return;
        };
        let preset = EffectMappingPreset {
            name: format!("{} Mapping Preset", current.inspection.name),
            source_format: current.inspection.source_format,
            target_format: target_format.get_untracked(),
            decisions: UserEffectDecisions {
                actions: decisions.get_untracked(),
            },
        };
        spawn_local(async move {
            match api::save_effect_preset(preset).await {
                Ok(Some(path)) => {
                    set_status_toast.set(Some(format!("Saved effect mapping preset to {path}")))
                }
                Ok(None) => {}
                Err(e) => set_error_msg.set(Some(e)),
            }
        });
    };

    let on_load_preset = move |_| {
        spawn_local(async move {
            match api::load_effect_preset().await {
                Ok(Some(preset)) => {
                    set_decisions.update(|map| {
                        for (k, v) in preset.decisions.actions {
                            map.insert(k, v);
                        }
                    });
                    set_status_toast.set(Some(format!("Loaded preset: {}", preset.name)));
                }
                Ok(None) => {}
                Err(e) => set_error_msg.set(Some(e)),
            }
        });
    };

    view! {
        <div class="app-shell">
            <header class="topbar">
                <div class="brand">
                    <div class="brand-mark">"AN"</div>
                    <div>
                        <div class="brand-title">"Aagedal NLE Project Converter"</div>
                        <div class="brand-sub">"Direct Native Project Conversion · Rust + Tauri · GPL-3.0"</div>
                    </div>
                </div>
                <div class="topbar-actions">
                    <button class="btn" disabled=move || busy.get() on:click=on_load_demo>
                        "Load Demo .prproj"
                    </button>
                    <button class="btn btn-primary" disabled=move || busy.get() on:click=on_open_file>
                        "Open Project..."
                    </button>
                </div>
            </header>

            <main class="main-content">
                {move || error_msg.get().map(|err| view! {
                    <div class="banner banner-error">
                        <span>{err}</span>
                        <button class="btn btn-sm" on:click=move |_| set_error_msg.set(None)>"Dismiss"</button>
                    </div>
                })}

                {move || status_toast.get().map(|msg| view! {
                    <div class="banner banner-success">
                        <span>{msg}</span>
                        <button class="btn btn-sm" on:click=move |_| set_status_toast.set(None)>"OK"</button>
                    </div>
                })}

                {move || match loaded.get() {
                    None => view! {
                        <section class="hero-card">
                            <h1>"Convert NLE Projects Directly — No XML Required"</h1>
                            <p>
                                "Read native Adobe Premiere Pro (.prproj) and DaVinci Resolve (.drp) project graphs directly in Rust, review app-exclusive effects interactively, and export ready-to-open native projects."
                            </p>
                            <div class="hero-actions">
                                <button class="btn btn-primary" disabled=move || busy.get() on:click=on_open_file>
                                    "Choose .prproj or .drp File"
                                </button>
                                <button class="btn" disabled=move || busy.get() on:click=on_load_demo>
                                    "Try Built-in Demo Project"
                                </button>
                            </div>
                            <div class="format-pills">
                                <span class="pill pill-purple">"Premiere .prproj → Resolve .drp (Native)"</span>
                                <span class="pill pill-cyan">"Resolve .drp → FCPXML / OTIO / XML"</span>
                                <span class="pill pill-amber">"Interactive Effect Mapping & Review Markers"</span>
                                <span class="pill pill-green">"100% Local · Free & Open Source"</span>
                            </div>
                        </section>
                    }.into_any(),
                    Some(payload) => {
                        let insp = payload.inspection.clone();
                        let plan = payload.effect_plan.clone();
                        let color_label = insp.working_color_space.clone().unwrap_or_else(|| "Rec. 709 / Default".into());

                        view! {
                            <div>
                                // Conversion Report Banner (if converted)
                                {move || report.get().map(|rep| {
                                    let out_path = rep.output_path.clone();
                                    let out_path_reveal = out_path.clone();
                                    view! {
                                        <section class="panel" style="border-color: rgba(16,185,129,0.5);">
                                            <div class="panel-header">
                                                <div>
                                                    <h3 class="panel-title" style="color: #6ee7b7;">"Conversion Complete"</h3>
                                                    <div class="project-path">{out_path}</div>
                                                </div>
                                                <div style="display:flex; gap:8px;">
                                                    <button class="btn btn-primary btn-sm" on:click=move |_| {
                                                        let p = out_path_reveal.clone();
                                                        spawn_local(async move {
                                                            let _ = api::reveal_in_file_manager(p).await;
                                                        });
                                                    }>"Reveal in Finder"</button>
                                                    <button class="btn btn-sm" on:click=move |_| set_report.set(None)>"Hide"</button>
                                                </div>
                                            </div>
                                            <div class="report-grid">
                                                <div class="stat-box"><div class="stat-val">{rep.sequences_converted}</div><div class="stat-lbl">"Sequences"</div></div>
                                                <div class="stat-box"><div class="stat-val">{rep.video_clips_converted}</div><div class="stat-lbl">"Video Clips"</div></div>
                                                <div class="stat-box"><div class="stat-val">{rep.audio_clips_converted}</div><div class="stat-lbl">"Audio Clips"</div></div>
                                                <div class="stat-box"><div class="stat-val">{rep.bins_created}</div><div class="stat-lbl">"Bins"</div></div>
                                                <div class="stat-box"><div class="stat-val">{rep.grades_attached}</div><div class="stat-lbl">"DRX Grades"</div></div>
                                                <div class="stat-box"><div class="stat-val">{rep.markers_written}</div><div class="stat-lbl">"Markers"</div></div>
                                                <div class="stat-box"><div class="stat-val">{rep.effect_markers_injected}</div><div class="stat-lbl">"FX Markers"</div></div>
                                            </div>
                                            {(!rep.messages.is_empty()).then(|| view! {
                                                <div class="log-list">
                                                    {rep.messages.into_iter().map(|m| {
                                                        let cls = match m.severity {
                                                            Severity::Info => "log-entry log-info",
                                                            Severity::Warning => "log-entry log-warning",
                                                            Severity::Error => "log-entry log-error",
                                                        };
                                                        let prefix = match (&m.sequence, &m.clip) {
                                                            (Some(s), Some(c)) => format!("[{s} · {c}] "),
                                                            (Some(s), None) => format!("[{s}] "),
                                                            _ => String::new(),
                                                        };
                                                        view! { <div class=cls>{format!("{}{} ({})", prefix, m.message, m.code)}</div> }
                                                    }).collect_view()}
                                                </div>
                                            })}
                                        </section>
                                    }
                                })}

                                // Top Target & Action Bar
                                <section class="convert-bar">
                                    <div class="project-meta">
                                        <div class="project-title-row">
                                            <h2>{insp.name.clone()}</h2>
                                            <span class="pill pill-purple">{insp.source_format.display_name()}</span>
                                            <span class="pill">{format!("Color: {color_label}")}</span>
                                        </div>
                                        <div class="project-path">
                                            {insp.source_path.clone().unwrap_or_default()}
                                        </div>
                                    </div>
                                    <div class="target-controls">
                                        <label style="font-size:12.5px; color:var(--muted);">"Convert to:"</label>
                                        <select
                                            prop:value=move || target_format.get().as_str()
                                            on:change=on_change_target
                                        >
                                            <option value="da_vinci_resolve">"DaVinci Resolve Native (.drp)"</option>
                                            <option value="final_cut_pro">"Final Cut Pro (.fcpxml)"</option>
                                            <option value="open_timeline_io">"OpenTimelineIO (.otio)"</option>
                                            <option value="fcp7_xml">"FCP7 XML (.xml — Premiere / Resolve)"</option>
                                        </select>
                                        <button
                                            class="btn btn-primary"
                                            disabled=move || busy.get() || selected_seq_ids.get().is_empty()
                                            on:click=on_convert
                                        >
                                            {move || if busy.get() { "Converting..." } else { "Convert & Save..." }}
                                        </button>
                                    </div>
                                </section>

                                <div class="workspace-grid">
                                    // LEFT COLUMN: Project summary, Sequences, Options, Media Relinker
                                    <div>
                                        <section class="panel">
                                            <div class="panel-header">
                                                <h3 class="panel-title">"Project Overview"</h3>
                                            </div>
                                            <div class="stat-row">
                                                <div class="stat-box">
                                                    <div class="stat-val">{insp.sequences.len()}</div>
                                                    <div class="stat-lbl">"Sequences"</div>
                                                </div>
                                                <div class="stat-box">
                                                    <div class="stat-val">{insp.bin_count}</div>
                                                    <div class="stat-lbl">"Bins"</div>
                                                </div>
                                                <div class="stat-box">
                                                    <div class="stat-val">{insp.media_count}</div>
                                                    <div class="stat-lbl">"Media"</div>
                                                </div>
                                            </div>
                                            {(insp.missing_media_count > 0).then(|| view! {
                                                <div style="font-size:12px; color:#fcd34d; margin-bottom:10px;">
                                                    {format!("{} media files not found at original paths — use Path Relinker below if drives moved (offline conversion still works).", insp.missing_media_count)}
                                                </div>
                                            })}
                                            <div class="panel-header" style="margin-top:10px;">
                                                <h3 class="panel-title">"Sequences to Convert"</h3>
                                            </div>
                                            {insp.sequences.iter().cloned().map(|seq| {
                                                let sid = seq.id.clone();
                                                let sid_chk = sid.clone();
                                                let sid_chg = sid.clone();
                                                view! {
                                                    <label class="seq-item">
                                                        <input
                                                            type="checkbox"
                                                            prop:checked=move || selected_seq_ids.get().contains(&sid_chk)
                                                            on:change=move |ev| {
                                                                let checked = event_target_checked(&ev);
                                                                let mut next = selected_seq_ids.get_untracked();
                                                                if checked {
                                                                    next.insert(sid_chg.clone());
                                                                } else {
                                                                    next.remove(&sid_chg);
                                                                }
                                                                let vec_ids: Vec<String> = next.iter().cloned().collect();
                                                                set_selected_seq_ids.set(next);
                                                                trigger_reanalyze(vec_ids, target_format.get_untracked());
                                                            }
                                                        />
                                                        <div class="seq-info">
                                                            <div class="seq-name">{seq.name}</div>
                                                            <div class="seq-specs">
                                                                {format!("{}×{} · {} fps · {}", seq.width, seq.height, seq.fps_label, seq.duration_tc)}
                                                            </div>
                                                            <div class="seq-specs">
                                                                {format!("V:{} ({} clips) · A:{} ({} clips) · {} mk · {} fx",
                                                                    seq.video_track_count, seq.video_clip_count,
                                                                    seq.audio_track_count, seq.audio_clip_count,
                                                                    seq.marker_count, seq.effect_count)}
                                                            </div>
                                                        </div>
                                                    </label>
                                                }
                                            }).collect_view()}
                                        </section>

                                        <section class="panel">
                                            <div class="panel-header">
                                                <h3 class="panel-title">"Conversion Options"</h3>
                                            </div>
                                            <div class="option-list">
                                                <label class="checkbox-row">
                                                    <input type="checkbox" prop:checked=move || include_bins.get() on:change=move |ev| set_include_bins.set(event_target_checked(&ev)) />
                                                    "Preserve nested project bins (Media Pool folders)"
                                                </label>
                                                <label class="checkbox-row">
                                                    <input type="checkbox" prop:checked=move || include_lumetri.get() on:change=move |ev| set_include_lumetri.set(event_target_checked(&ev)) />
                                                    "Translate Lumetri Color → Resolve DRX serial nodes"
                                                </label>
                                                <label class="checkbox-row">
                                                    <input type="checkbox" prop:checked=move || include_markers.get() on:change=move |ev| set_include_markers.set(event_target_checked(&ev)) />
                                                    "Preserve sequence timeline markers"
                                                </label>
                                                <label class="checkbox-row">
                                                    <input type="checkbox" prop:checked=move || include_clip_colors.get() on:change=move |ev| set_include_clip_colors.set(event_target_checked(&ev)) />
                                                    "Map Premiere clip label colors → Resolve clip colors"
                                                </label>
                                                <label class="checkbox-row">
                                                    <input type="checkbox" prop:checked=move || inject_fx_markers.get() on:change=move |ev| set_inject_fx_markers.set(event_target_checked(&ev)) />
                                                    "Inject color-coded timeline markers for mapped/unmapped effects"
                                                </label>
                                            </div>
                                        </section>

                                        <section class="panel">
                                            <div class="panel-header">
                                                <h3 class="panel-title">"Media Path Relinker (Optional)"</h3>
                                            </div>
                                            <div style="font-size:12px; color:var(--muted);">
                                                "Rewrite drive/folder prefixes when migrating across workstations (e.g. D:\\Media → /Volumes/RAID/Media)."
                                            </div>
                                            <div class="relink-grid">
                                                <input
                                                    type="text"
                                                    placeholder="From prefix (e.g. /Media)"
                                                    prop:value=move || relink_from.get()
                                                    on:input=move |ev| set_relink_from.set(event_target_value(&ev))
                                                />
                                                <input
                                                    type="text"
                                                    placeholder="To prefix (e.g. /Volumes/Edit)"
                                                    prop:value=move || relink_to.get()
                                                    on:input=move |ev| set_relink_to.set(event_target_value(&ev))
                                                />
                                            </div>
                                        </section>
                                    </div>

                                    // RIGHT COLUMN: Interactive Effect Mapping Review
                                    <div>
                                        <section class="panel">
                                            <div class="panel-header">
                                                <div>
                                                    <h3 class="panel-title">"Interactive Effect Translation & Mapping"</h3>
                                                    <div style="font-size:12px; color:var(--muted); margin-top:2px;">
                                                        "App-exclusive effects are detected automatically so you can choose corresponding target NLE equivalents or place annotated review markers."
                                                    </div>
                                                </div>
                                                <div style="display:flex; gap:8px;">
                                                    <button class="btn btn-sm" on:click=on_load_preset>"Load Preset"</button>
                                                    <button class="btn btn-sm" on:click=on_save_preset>"Save Preset"</button>
                                                </div>
                                            </div>

                                            <div style="display:flex; gap:8px; flex-wrap:wrap; margin-bottom:14px;">
                                                <span class="pill pill-amber">{format!("{} Ask User / Suggested", plan.needs_user_review_count)}</span>
                                                <span class="pill pill-red">{format!("{} Unmapped / 3rd Party", plan.unmapped_count)}</span>
                                                <span class="pill pill-green">{format!("{} Native Auto-Convert", plan.native_auto_count)}</span>
                                            </div>

                                            {if plan.effects.is_empty() {
                                                view! {
                                                    <div style="padding:24px; text-align:center; color:var(--muted); background:var(--surface-2); border-radius:10px;">
                                                        "No non-default or app-exclusive effects detected in the selected sequences."
                                                    </div>
                                                }.into_any()
                                            } else {
                                                plan.effects.into_iter().map(|fx| {
                                                    let card_cls = match fx.fidelity {
                                                        MappingFidelity::AskUserSuggested => "fx-card fx-card-ask",
                                                        MappingFidelity::Unmapped => "fx-card fx-card-unmapped",
                                                        MappingFidelity::NativeAutomatic => "fx-card fx-card-native",
                                                    };
                                                    let (badge_cls, badge_txt) = match fx.fidelity {
                                                        MappingFidelity::AskUserSuggested => ("pill pill-amber", "Ask User · Equivalent Available"),
                                                        MappingFidelity::Unmapped => ("pill pill-red", "Unmapped / Plugin"),
                                                        MappingFidelity::NativeAutomatic => ("pill pill-green", "Native Auto-Convert"),
                                                    };

                                                    let fx_key = fx.key.clone();
                                                    let fx_key_val = fx_key.clone();
                                                    let fx_key_chg = fx_key.clone();
                                                    let targets_for_select = fx.available_targets.clone();
                                                    let targets_for_change = fx.available_targets.clone();
                                                    let fidelity = fx.fidelity;
                                                    let sample_line = if fx.sample_clips.is_empty() {
                                                        String::new()
                                                    } else {
                                                        format!("Clips: {}", fx.sample_clips.join(", "))
                                                    };

                                                    view! {
                                                        <div class=card_cls>
                                                            <div class="fx-top">
                                                                <div>
                                                                    <div class="fx-name">
                                                                        {fx.display_name.clone()}
                                                                        <span style="font-weight:400; color:var(--muted); margin-left:8px; font-size:12px;">
                                                                            {format!("×{} clips", fx.clip_count)}
                                                                        </span>
                                                                    </div>
                                                                    <div class="fx-match">
                                                                        {fx.match_name.clone()}
                                                                        {fx.has_keyframes.then(|| " · Animated (keyframes)")}
                                                                    </div>
                                                                </div>
                                                                <span class=badge_cls>{badge_txt}</span>
                                                            </div>

                                                            <div class="fx-notes">{fx.notes.clone()}</div>

                                                            <div class="fx-controls">
                                                                <select
                                                                    prop:value=move || {
                                                                        match decisions.get().get(&fx_key_val) {
                                                                            Some(MappingAction::NativeConvert) => "native".to_string(),
                                                                            Some(MappingAction::MapToEquivalent { target_effect_id, .. }) => format!("target:{target_effect_id}"),
                                                                            Some(MappingAction::PlaceMarkerOnly) => "marker".to_string(),
                                                                            Some(MappingAction::Skip) => "skip".to_string(),
                                                                            None => "marker".to_string(),
                                                                        }
                                                                    }
                                                                    on:change=move |ev| {
                                                                        let v = event_target_value(&ev);
                                                                        let action = if v == "native" {
                                                                            MappingAction::NativeConvert
                                                                        } else if v == "marker" {
                                                                            MappingAction::PlaceMarkerOnly
                                                                        } else if v == "skip" {
                                                                            MappingAction::Skip
                                                                        } else if let Some(tid) = v.strip_prefix("target:") {
                                                                            let tname = targets_for_change
                                                                                .iter()
                                                                                .find(|t| t.id == tid)
                                                                                .map(|t| t.display_name.clone())
                                                                                .unwrap_or_else(|| tid.to_string());
                                                                            MappingAction::MapToEquivalent {
                                                                                target_effect_id: tid.to_string(),
                                                                                target_display_name: tname,
                                                                                add_review_marker: true,
                                                                            }
                                                                        } else {
                                                                            MappingAction::PlaceMarkerOnly
                                                                        };
                                                                        set_decisions.update(|map| {
                                                                            map.insert(fx_key_chg.clone(), action);
                                                                        });
                                                                    }
                                                                >
                                                                    {(fidelity == MappingFidelity::NativeAutomatic).then(|| view! {
                                                                        <option value="native">"Convert Natively into Output Project (Recommended)"</option>
                                                                    })}
                                                                    {targets_for_select.into_iter().map(|t| {
                                                                        let opt_val = format!("target:{}", t.id);
                                                                        let studio = if t.studio_only { " [Studio]" } else { "" };
                                                                        let label = format!("Map → {}{studio}", t.display_name);
                                                                        view! { <option value=opt_val>{label}</option> }
                                                                    }).collect_view()}
                                                                    <option value="marker">"Drop Effect + Place Timeline Marker with Parameters"</option>
                                                                    <option value="skip">"Skip Effect Completely"</option>
                                                                </select>
                                                            </div>

                                                            {(!sample_line.is_empty()).then(|| view! {
                                                                <div class="fx-clips">{sample_line}</div>
                                                            })}
                                                        </div>
                                                    }
                                                }).collect_view().into_any()
                                            }}
                                        </section>
                                    </div>
                                </div>
                            </div>
                        }.into_any()
                    }
                }}
            </main>
        </div>
    }
}
