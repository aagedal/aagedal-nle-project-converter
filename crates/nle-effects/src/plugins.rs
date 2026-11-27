use nle_core::{NleFormat, TrackKind};
use roxmltree::{Document, Node, ParsingOptions};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Native plugin host format discovered on the local workstation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginFormat {
    PremiereMediaCore,
    OpenFx,
    FxPlug,
    MotionEffectTemplate,
    AudioUnit,
    Vst3,
}

impl PluginFormat {
    pub fn short_label(self) -> &'static str {
        match self {
            Self::PremiereMediaCore => "Premiere Plugin",
            Self::OpenFx => "Resolve OFX",
            Self::FxPlug => "FCP FxPlug",
            Self::MotionEffectTemplate => "FCP Motion Effect",
            Self::AudioUnit => "Audio Unit (AU)",
            Self::Vst3 => "VST3",
        }
    }

    /// True if our writer attaches this plugin natively onto clips in the target project file.
    pub fn supports_direct_attachment(self, target_nle: NleFormat) -> bool {
        matches!(
            (self, target_nle),
            (Self::PremiereMediaCore, NleFormat::PremierePro)
                | (Self::FxPlug | Self::MotionEffectTemplate, NleFormat::FinalCutPro)
        )
    }
}

/// One locally installed third-party plugin discovered on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub name: String,
    pub normalized_name: String,
    pub target_nle: NleFormat,
    pub format: PluginFormat,
    pub media_kind: TrackKind,
    /// Native host identifier written into target projects:
    /// - Premiere: `AE.<PiPL eMNA>` (e.g. `AE.AGDL Multi-Layer Anonymizer`)
    /// - Resolve OFX: OpenFX bundle/plugin ID (e.g. `me.aagedal.ofx.MultiLayerAnonymizer`)
    /// - Final Cut Pro FxPlug: `FxPlug:<UUID>`
    /// - Final Cut Pro Motion Effect: `~/Effects.localized/.../*.moef`
    /// - Audio Unit / VST3: bundle identifier or plugin name
    pub native_id: String,
    pub bundle_id: Option<String>,
    pub category: Option<String>,
    pub version: Option<String>,
    pub path: String,
    pub match_aliases: Vec<String>,
}

impl InstalledPlugin {
    /// Encode a `TargetEffectOption.id` prefix so `apply_effect_decisions` knows to rewrite
    /// `EffectInstance.match_name` to this plugin's native target ID.
    pub fn target_option_id(&self) -> String {
        let prefix = match (self.target_nle, self.format) {
            (NleFormat::PremierePro, PluginFormat::PremiereMediaCore) => "plugin.premiere",
            (NleFormat::FinalCutPro, PluginFormat::FxPlug | PluginFormat::MotionEffectTemplate) => {
                "plugin.fcp"
            }
            (NleFormat::DaVinciResolve, PluginFormat::OpenFx) => "plugin.resolve.ofx",
            (_, PluginFormat::AudioUnit) => "plugin.audio.au",
            (_, PluginFormat::Vst3) => "plugin.audio.vst3",
            _ => "plugin.other",
        };
        format!("{prefix}:{}", self.native_id)
    }

    pub fn target_display_label(&self) -> String {
        let ver = self
            .version
            .as_deref()
            .filter(|v| !v.is_empty())
            .map(|v| format!(" v{v}"))
            .unwrap_or_default();
        format!(
            "{} (Installed {}{ver})",
            self.name,
            self.format.short_label()
        )
    }
}

/// Summary of one third-party plugin grouped across all NLEs where it is installed locally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossNlePluginSummary {
    pub name: String,
    pub normalized_name: String,
    pub media_kind: TrackKind,
    pub installations: Vec<InstalledPlugin>,
}

/// Registry of third-party plugins installed on the local machine across Premiere Pro,
/// DaVinci Resolve (OFX), Final Cut Pro (FxPlug / Motion Templates), and AU / VST3 audio hosts.
#[derive(Debug, Clone, Default)]
pub struct InstalledPluginRegistry {
    plugins: Vec<InstalledPlugin>,
}

static SYSTEM_REGISTRY: OnceLock<InstalledPluginRegistry> = OnceLock::new();

impl InstalledPluginRegistry {
    /// Cached system-wide scan of locally installed NLE plugins.
    pub fn system() -> &'static Self {
        SYSTEM_REGISTRY.get_or_init(Self::scan_system)
    }

    /// Perform a fresh scan of standard OS plugin directories.
    pub fn scan_system() -> Self {
        let mut plugins = Vec::new();
        let home = std::env::var_os("HOME").map(PathBuf::from);

        #[cfg(target_os = "macos")]
        {
            // 1. Premiere Pro MediaCore video plugins (.plugin)
            let mut premiere_dirs = vec![PathBuf::from(
                "/Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore",
            )];
            if let Some(ref h) = home {
                premiere_dirs
                    .push(h.join("Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore"));
            }
            for dir in premiere_dirs {
                scan_premiere_mediacore_dir(&dir, &mut plugins);
            }

            // 2. DaVinci Resolve OpenFX (.ofx.bundle)
            let mut ofx_dirs = vec![PathBuf::from("/Library/OFX/Plugins")];
            if let Some(ref h) = home {
                ofx_dirs.push(h.join("Library/OFX/Plugins"));
            }
            for dir in ofx_dirs {
                scan_ofx_dir(&dir, 0, &mut plugins);
            }

            // 3. Final Cut Pro FxPlug (.fxplug / .pluginkit / .appex)
            let mut fxplug_dirs = vec![PathBuf::from("/Library/Plug-Ins/FxPlug")];
            if let Some(ref h) = home {
                fxplug_dirs.push(h.join("Library/Plug-Ins/FxPlug"));
            }
            for dir in fxplug_dirs {
                scan_fxplug_dir(&dir, &mut plugins);
            }
            // FxPlug4 XPC extensions live inside host .app/Contents/PlugIns/*.pluginkit
            let mut app_dirs = vec![PathBuf::from("/Applications")];
            if let Some(ref h) = home {
                app_dirs.push(h.join("Applications"));
            }
            for app_dir in app_dirs {
                scan_applications_for_fxplug(&app_dir, &mut plugins);
            }

            // 4. Final Cut Pro Motion Effect templates (.moef)
            let mut motion_dirs = vec![PathBuf::from(
                "/Library/Application Support/Final Cut Pro/Templates.localized/Effects.localized",
            )];
            if let Some(ref h) = home {
                motion_dirs.push(h.join("Movies/Motion Templates.localized/Effects.localized"));
            }
            for dir in motion_dirs {
                scan_motion_effects_dir(&dir, &mut plugins);
            }

            // 5. Cross-NLE Audio Units (.component -> Premiere, Resolve, Final Cut Pro)
            let mut au_dirs = vec![PathBuf::from("/Library/Audio/Plug-Ins/Components")];
            if let Some(ref h) = home {
                au_dirs.push(h.join("Library/Audio/Plug-Ins/Components"));
            }
            for dir in au_dirs {
                scan_audio_plugins_dir(&dir, "component", PluginFormat::AudioUnit, &mut plugins);
            }

            // 6. Cross-NLE VST3 (.vst3 -> Premiere, Resolve)
            let mut vst3_dirs = vec![PathBuf::from("/Library/Audio/Plug-Ins/VST3")];
            if let Some(ref h) = home {
                vst3_dirs.push(h.join("Library/Audio/Plug-Ins/VST3"));
            }
            for dir in vst3_dirs {
                scan_audio_plugins_dir(&dir, "vst3", PluginFormat::Vst3, &mut plugins);
            }
        }

        #[cfg(target_os = "windows")]
        {
            let _ = &home;
            scan_ofx_dir(
                Path::new(r"C:\Program Files\Common Files\OFX\Plugins"),
                0,
                &mut plugins,
            );
            scan_audio_plugins_dir(
                Path::new(r"C:\Program Files\Common Files\VST3"),
                "vst3",
                PluginFormat::Vst3,
                &mut plugins,
            );
        }

        #[cfg(target_os = "linux")]
        {
            let _ = &home;
            scan_ofx_dir(Path::new("/usr/OFX/Plugins"), 0, &mut plugins);
        }

        Self::from_plugins(plugins)
    }

    /// Construct a registry from an explicit list of plugins (useful for tests or custom inventories).
    pub fn from_plugins(plugins: Vec<InstalledPlugin>) -> Self {
        let mut deduped = Vec::new();
        let mut seen: HashSet<(NleFormat, PluginFormat, String)> = HashSet::new();
        for p in plugins {
            let key = (p.target_nle, p.format, p.native_id.to_ascii_lowercase());
            if seen.insert(key) {
                deduped.push(p);
            }
        }
        deduped.sort_by(|a, b| {
            a.normalized_name
                .cmp(&b.normalized_name)
                .then_with(|| a.target_nle.short_name().cmp(b.target_nle.short_name()))
        });
        Self { plugins: deduped }
    }

    pub fn plugins(&self) -> &[InstalledPlugin] {
        &self.plugins
    }

    /// Group installed plugins by normalized name and return every plugin available in `min_nles`
    /// or more distinct NLEs (pass `min_nles = 2` for cross-NLE plugins, or `1` for all plugins).
    pub fn grouped_plugins(&self, min_nles: usize) -> Vec<CrossNlePluginSummary> {
        let mut by_key: BTreeMap<(TrackKindKey, String), Vec<InstalledPlugin>> = BTreeMap::new();
        for p in &self.plugins {
            by_key
                .entry((TrackKindKey::from(p.media_kind), p.normalized_name.clone()))
                .or_default()
                .push(p.clone());
        }

        let mut out = Vec::new();
        for ((_, norm), installs) in by_key {
            let distinct_nles: HashSet<NleFormat> = installs.iter().map(|i| i.target_nle).collect();
            if distinct_nles.len() < min_nles {
                continue;
            }
            let display_name = installs
                .iter()
                .map(|i| i.name.as_str())
                .min_by_key(|s| s.len())
                .unwrap_or(&norm)
                .to_string();
            let media_kind = installs[0].media_kind;
            out.push(CrossNlePluginSummary {
                name: display_name,
                normalized_name: norm,
                media_kind,
                installations: installs,
            });
        }
        out
    }

    /// Find locally installed plugins for `target_format` that correspond to the same third-party
    /// plugin identified by `(source_format, match_name, display_name, media_kind)`.
    pub fn find_target_matches(
        &self,
        source_format: NleFormat,
        target_format: NleFormat,
        match_name: &str,
        display_name: &str,
        media_kind: TrackKind,
    ) -> Vec<InstalledPlugin> {
        if !matches!(
            target_format,
            NleFormat::PremierePro | NleFormat::DaVinciResolve | NleFormat::FinalCutPro
        ) {
            return Vec::new();
        }

        // 1. Collect normalized candidate names/aliases from the effect itself
        let mut query_names: HashSet<String> = HashSet::new();
        let mut query_aliases: HashSet<String> = HashSet::new();
        collect_query_tokens(match_name, display_name, &mut query_names, &mut query_aliases);

        // 2. If the source plugin itself is installed locally for `source_format`, enrich the query
        //    with its canonical display name & aliases (e.g. `AE.AGDL Multi-Layer Anonymizer` ->
        //    `"redact anonymizer"` + `"multilayeranonymizer"`).
        for src in self
            .plugins
            .iter()
            .filter(|p| p.target_nle == source_format && p.media_kind == media_kind)
        {
            if plugin_matches_source(src, match_name, &query_names, &query_aliases) {
                if !src.normalized_name.is_empty() {
                    query_names.insert(src.normalized_name.clone());
                }
                for a in &src.match_aliases {
                    if !a.is_empty() {
                        query_aliases.insert(a.clone());
                    }
                }
            }
        }

        // 3. Score candidate installed plugins for `target_format`
        let mut scored: Vec<(u32, InstalledPlugin)> = Vec::new();
        for cand in self
            .plugins
            .iter()
            .filter(|p| p.target_nle == target_format && p.media_kind == media_kind)
        {
            let score = score_candidate(cand, &query_names, &query_aliases);
            if score > 0 {
                scored.push((score, cand.clone()));
            }
        }

        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| {
                    format_priority(b.1.format).cmp(&format_priority(a.1.format))
                })
                .then_with(|| a.1.name.cmp(&b.1.name))
        });

        scored.into_iter().map(|(_, p)| p).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TrackKindKey {
    Video,
    Audio,
}

impl From<TrackKind> for TrackKindKey {
    fn from(k: TrackKind) -> Self {
        match k {
            TrackKind::Video => Self::Video,
            TrackKind::Audio => Self::Audio,
        }
    }
}

fn format_priority(fmt: PluginFormat) -> u8 {
    match fmt {
        PluginFormat::PremiereMediaCore => 5,
        PluginFormat::OpenFx => 5,
        PluginFormat::FxPlug => 5,
        PluginFormat::MotionEffectTemplate => 4,
        PluginFormat::AudioUnit => 3,
        PluginFormat::Vst3 => 2,
    }
}

fn plugin_matches_source(
    src: &InstalledPlugin,
    raw_match_name: &str,
    query_names: &HashSet<String>,
    query_aliases: &HashSet<String>,
) -> bool {
    let raw_lower = raw_match_name.to_ascii_lowercase();
    let nid_lower = src.native_id.to_ascii_lowercase();
    if !nid_lower.is_empty() && (raw_lower == nid_lower || raw_lower.contains(&nid_lower)) {
        return true;
    }
    if let Some(bid) = &src.bundle_id {
        let bid_lower = bid.to_ascii_lowercase();
        if !bid_lower.is_empty() && raw_lower.contains(&bid_lower) {
            return true;
        }
    }
    if query_names.contains(&src.normalized_name) {
        return true;
    }
    src.match_aliases
        .iter()
        .any(|a| query_names.contains(a) || query_aliases.contains(a))
}

fn score_candidate(
    cand: &InstalledPlugin,
    query_names: &HashSet<String>,
    query_aliases: &HashSet<String>,
) -> u32 {
    if query_names.contains(&cand.normalized_name) {
        return 100;
    }
    if cand
        .match_aliases
        .iter()
        .any(|a| query_names.contains(a) || query_aliases.contains(a))
    {
        return 90;
    }
    // Multi-word / distinctive compact-slug match (avoid matching single generic words like "blur")
    let cand_slug = compact_slug(&cand.normalized_name);
    if cand_slug.len() >= 8 {
        for q in query_names.iter().chain(query_aliases.iter()) {
            let q_slug = compact_slug(q);
            if q_slug.len() >= 8
                && (cand_slug == q_slug
                    || (cand_slug.contains(&q_slug) && q.contains(' '))
                    || (q_slug.contains(&cand_slug) && cand.normalized_name.contains(' ')))
            {
                return 75;
            }
        }
    }
    0
}

fn collect_query_tokens(
    match_name: &str,
    display_name: &str,
    names: &mut HashSet<String>,
    aliases: &mut HashSet<String>,
) {
    let norm_disp = normalize_plugin_name(display_name);
    if is_meaningful_name(&norm_disp) {
        names.insert(norm_disp.clone());
        let slug = compact_slug(&norm_disp);
        if slug.len() >= 6 {
            aliases.insert(slug);
        }
    }

    // Parse structured match_name formats:
    // - "AE.AGDL Multi-Layer Anonymizer"
    // - "fcp.Redact Anonymizer:FxPlug:211035B9-..."
    // - "fcp.Redact Anonymizer:~/Effects.localized/Aagedal/Redact Anonymizer/Redact Anonymizer.moef"
    // - "ofx:me.aagedal.ofx.MultiLayerAnonymizer"
    let stripped = match_name
        .trim()
        .strip_prefix("AE.")
        .or_else(|| match_name.trim().strip_prefix("ae."))
        .or_else(|| match_name.trim().strip_prefix("ofx:"))
        .unwrap_or(match_name.trim());

    if let Some(rest) = stripped
        .strip_prefix("fcp.")
        .or_else(|| stripped.strip_prefix("FCP."))
    {
        if let Some((left, right)) = rest.split_once(':') {
            let n = normalize_plugin_name(left);
            if is_meaningful_name(&n) {
                names.insert(n);
            }
            // Extract template stem if right is a .moef path
            if let Some(stem) = Path::new(right).file_stem().and_then(|s| s.to_str()) {
                let ns = normalize_plugin_name(stem);
                if is_meaningful_name(&ns) {
                    names.insert(ns);
                }
            }
            let r_lower = right.trim().to_ascii_lowercase();
            if !r_lower.is_empty() {
                aliases.insert(r_lower);
            }
        } else {
            let n = normalize_plugin_name(rest);
            if is_meaningful_name(&n) {
                names.insert(n);
            }
        }
    } else {
        let nm = normalize_plugin_name(stripped);
        if is_meaningful_name(&nm) {
            aliases.insert(nm.clone());
            let slug = compact_slug(&nm);
            if slug.len() >= 6 {
                aliases.insert(slug);
            }
        }
        // Reverse-DNS bundle ID tail (e.g. "me.aagedal.ofx.MultiLayerAnonymizer" -> "multilayeranonymizer")
        if let Some(tail) = stripped.rsplit('.').next() {
            let t = compact_slug(tail);
            if t.len() >= 6 {
                aliases.insert(t);
            }
        }
    }
}

fn is_meaningful_name(s: &str) -> bool {
    s.len() >= 3
        && !matches!(
            s,
            "effect" | "filter" | "plugin" | "video" | "audio" | "transition" | "blur" | "color"
        )
}

/// Normalize a plugin display name for cross-NLE matching (strips host-specific suffixes like
/// `"OFX"`, `"FxPlug"`, `"XPC Service"`, `"Plugin"`, `"VST3"`, `"AU"`).
pub fn normalize_plugin_name(raw: &str) -> String {
    let mut s = raw
        .trim()
        .replace(['_', '-', '/'], " ")
        .to_ascii_lowercase();
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    const SUFFIXES: &[&str] = &[
        " xpc service",
        " openfx",
        " ofx",
        " fxplug",
        " audio unit",
        " vst3",
        " vst",
        " plugin",
        " plug in",
        " au",
    ];
    loop {
        let mut changed = false;
        for suf in SUFFIXES {
            if let Some(trimmed) = s.strip_suffix(suf) {
                let t = trimmed.trim();
                if !t.is_empty() {
                    s = t.to_string();
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    s
}

fn compact_slug(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn build_aliases(
    name: &str,
    normalized_name: &str,
    extra: &[Option<&str>],
    bundle_id: Option<&str>,
) -> Vec<String> {
    let mut set = HashSet::new();
    if is_meaningful_name(normalized_name) {
        set.insert(normalized_name.to_string());
    }
    let name_slug = compact_slug(normalized_name);
    if name_slug.len() >= 6 {
        set.insert(name_slug);
    }
    let raw_slug = compact_slug(name);
    if raw_slug.len() >= 6 {
        set.insert(raw_slug);
    }
    for item in extra.iter().flatten() {
        let n = normalize_plugin_name(item);
        if is_meaningful_name(&n) {
            set.insert(n.clone());
        }
        let s = compact_slug(item);
        if s.len() >= 6 {
            set.insert(s);
        }
        // Strip leading 3-5 letter vendor acronym (e.g. "AGDL Multi-Layer Anonymizer" -> "multilayeranonymizer")
        if let Some((first, rest)) = item.trim().split_once(' ')
            && first.len() <= 5 && first.chars().all(|c| c.is_ascii_alphanumeric()) {
                let rs = compact_slug(rest);
                if rs.len() >= 6 {
                    set.insert(rs);
                }
            }
    }
    if let Some(bid) = bundle_id {
        let bid_lower = bid.trim().to_ascii_lowercase();
        if !bid_lower.is_empty() {
            set.insert(bid_lower.clone());
        }
        if let Some(tail) = bid.rsplit('.').next() {
            let ts = compact_slug(tail);
            if ts.len() >= 6 {
                set.insert(ts);
            }
        }
    }
    let mut out: Vec<String> = set.into_iter().collect();
    out.sort();
    out
}

// ── Premiere Pro MediaCore Scanner (.plugin + Mac PiPL .rsrc) ───────────

#[derive(Debug, Default)]
pub struct PiplMetadata {
    pub kind: Option<String>,
    pub name: Option<String>,
    pub category: Option<String>,
    pub match_name: Option<String>,
}

/// Pure-Rust parser for Adobe Mac PiPL (`8BIM`) resource files (`Contents/Resources/*.rsrc`).
pub fn parse_pipl_rsrc(data: &[u8]) -> Option<PiplMetadata> {
    let mut meta = PiplMetadata::default();
    let mut found_any = false;
    let mut idx = 0usize;

    while idx + 16 <= data.len() {
        let Some(rel) = data[idx..].windows(4).position(|w| w == b"8BIM") else {
            break;
        };
        let pos = idx + rel;
        if pos + 16 > data.len() {
            break;
        }
        let tag = &data[pos + 4..pos + 8];
        let prop_len =
            u32::from_be_bytes(data[pos + 12..pos + 16].try_into().ok()?) as usize;
        let val_start = pos + 16;
        if val_start + prop_len > data.len() {
            idx = pos + 4;
            continue;
        }
        let val = &data[val_start..val_start + prop_len];
        match tag {
            b"kind" if val.len() == 4 => {
                meta.kind = Some(String::from_utf8_lossy(val).into_owned());
                found_any = true;
            }
            b"name" => {
                if let Some(s) = decode_pascal_string(val) {
                    meta.name = Some(s);
                    found_any = true;
                }
            }
            b"catg" => {
                if let Some(s) = decode_pascal_string(val) {
                    meta.category = Some(s);
                    found_any = true;
                }
            }
            b"eMNA" => {
                if let Some(s) = decode_pascal_string(val) {
                    meta.match_name = Some(s);
                    found_any = true;
                }
            }
            _ => {}
        }
        let padded = (prop_len + 3) & !3;
        idx = val_start + padded;
    }

    if found_any {
        Some(meta)
    } else {
        None
    }
}

fn decode_pascal_string(val: &[u8]) -> Option<String> {
    let &len = val.first()?;
    let n = len as usize;
    if n == 0 || 1 + n > val.len() {
        return None;
    }
    let s = String::from_utf8_lossy(&val[1..1 + n])
        .trim_matches('\0')
        .trim()
        .to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(target_os = "macos")]
fn scan_premiere_mediacore_dir(dir: &Path, out: &mut Vec<InstalledPlugin>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "plugin" {
            continue;
        }
        let plist = read_xml_plist_file(&path.join("Contents/Info.plist"));
        let pkg_type = plist
            .as_ref()
            .and_then(|p| p.str_val("CFBundlePackageType"))
            .unwrap_or("");

        // Inspect Contents/Resources/*.rsrc for PiPL 8BIM metadata
        let mut pipl: Option<PiplMetadata> = None;
        if let Ok(res_entries) = fs::read_dir(path.join("Contents/Resources")) {
            for re in res_entries.flatten() {
                let rp = re.path();
                if rp
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("rsrc"))
                    && let Ok(bytes) = fs::read(&rp)
                        && let Some(parsed) = parse_pipl_rsrc(&bytes) {
                            pipl = Some(parsed);
                            break;
                        }
            }
        }

        // Only collect video effect filters (`eFKT`) — skip I/O hardware & codec plugins
        let is_effect = pipl
            .as_ref()
            .and_then(|p| p.kind.as_deref())
            .is_some_and(|k| k == "eFKT")
            || pkg_type == "eFKT";
        if !is_effect {
            continue;
        }

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Plugin");
        let name = pipl
            .as_ref()
            .and_then(|p| p.name.clone())
            .or_else(|| {
                plist.as_ref().and_then(|p| {
                    p.str_val("CFBundleDisplayName")
                        .or_else(|| p.str_val("CFBundleName"))
                        .map(|s| s.to_string())
                })
            })
            .unwrap_or_else(|| stem.to_string());
        let emna = pipl.as_ref().and_then(|p| p.match_name.as_deref());
        let native_id = match emna {
            Some(m) if m.starts_with("AE.") => m.to_string(),
            Some(m) => format!("AE.{m}"),
            None => format!("AE.{name}"),
        };
        let bundle_id = plist
            .as_ref()
            .and_then(|p| p.str_val("CFBundleIdentifier"))
            .map(|s| s.to_string());
        let version = plist
            .as_ref()
            .and_then(|p| {
                p.str_val("CFBundleShortVersionString")
                    .or_else(|| p.str_val("CFBundleVersion"))
            })
            .map(|s| s.to_string());
        let category = pipl.as_ref().and_then(|p| p.category.clone());
        let normalized_name = normalize_plugin_name(&name);
        let match_aliases = build_aliases(
            &name,
            &normalized_name,
            &[emna, Some(stem)],
            bundle_id.as_deref(),
        );

        out.push(InstalledPlugin {
            name,
            normalized_name,
            target_nle: NleFormat::PremierePro,
            format: PluginFormat::PremiereMediaCore,
            media_kind: TrackKind::Video,
            native_id,
            bundle_id,
            category,
            version,
            path: path.to_string_lossy().into_owned(),
            match_aliases,
        });
    }
}

// ── DaVinci Resolve OpenFX Scanner (.ofx.bundle) ────────────────────────

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn scan_ofx_dir(dir: &Path, depth: usize, out: &mut Vec<InstalledPlugin>) {
    if depth > 2 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let fname = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if fname.to_ascii_lowercase().ends_with(".ofx.bundle") {
            let plist = read_xml_plist_file(&path.join("Contents/Info.plist"));
            let stem = fname
                .strip_suffix(".ofx.bundle")
                .or_else(|| fname.strip_suffix(".OFX.bundle"))
                .unwrap_or(fname);
            let raw_name = plist
                .as_ref()
                .and_then(|p| {
                    p.str_val("CFBundleDisplayName")
                        .or_else(|| p.str_val("CFBundleName"))
                })
                .unwrap_or(stem);
            let display_name = raw_name
                .strip_suffix(" OFX")
                .or_else(|| raw_name.strip_suffix(" OpenFX"))
                .unwrap_or(raw_name)
                .trim()
                .to_string();
            let bundle_id = plist
                .as_ref()
                .and_then(|p| p.str_val("CFBundleIdentifier"))
                .map(|s| s.to_string());
            let version = plist
                .as_ref()
                .and_then(|p| {
                    p.str_val("CFBundleShortVersionString")
                        .or_else(|| p.str_val("CFBundleVersion"))
                })
                .map(|s| s.to_string());
            let native_id = bundle_id
                .clone()
                .unwrap_or_else(|| format!("ofx.{stem}"));
            let normalized_name = normalize_plugin_name(&display_name);
            let match_aliases = build_aliases(
                &display_name,
                &normalized_name,
                &[Some(raw_name), Some(stem)],
                bundle_id.as_deref(),
            );

            out.push(InstalledPlugin {
                name: display_name,
                normalized_name,
                target_nle: NleFormat::DaVinciResolve,
                format: PluginFormat::OpenFx,
                media_kind: TrackKind::Video,
                native_id,
                bundle_id,
                category: None,
                version,
                path: path.to_string_lossy().into_owned(),
                match_aliases,
            });
        } else {
            scan_ofx_dir(&path, depth + 1, out);
        }
    }
}

// ── Final Cut Pro FxPlug & Motion Effect Scanners ───────────────────────

#[cfg(target_os = "macos")]
fn scan_applications_for_fxplug(apps_dir: &Path, out: &mut Vec<InstalledPlugin>) {
    let Ok(entries) = fs::read_dir(apps_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let app_path = entry.path();
        if app_path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("app"))
        {
            let plugins_dir = app_path.join("Contents/PlugIns");
            if plugins_dir.is_dir() {
                scan_fxplug_dir(&plugins_dir, out);
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn scan_fxplug_dir(dir: &Path, out: &mut Vec<InstalledPlugin>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(ext.as_str(), "pluginkit" | "fxplug" | "appex") {
            continue;
        }
        let Some(plist) = read_xml_plist_file(&path.join("Contents/Info.plist")) else {
            continue;
        };
        extract_fxplug_entries_from_plist(&plist, &path, out);
    }
}

/// Extract FxPlug plugin entries from a parsed `Info.plist` (`ProPlugPlugInList`).
pub fn extract_fxplug_entries_from_plist_xml(
    xml: &str,
    bundle_path: &Path,
) -> Vec<InstalledPlugin> {
    let mut out = Vec::new();
    if let Some(plist) = parse_xml_plist(xml) {
        extract_fxplug_entries_from_plist(&plist, bundle_path, &mut out);
    }
    out
}

fn extract_fxplug_entries_from_plist(
    plist: &XmlPlistDict,
    bundle_path: &Path,
    out: &mut Vec<InstalledPlugin>,
) {
    let Some(plugin_list) = plist.array_of_dicts("ProPlugPlugInList") else {
        return;
    };
    let mut group_names: BTreeMap<String, String> = BTreeMap::new();
    if let Some(groups) = plist.array_of_dicts("ProPlugPlugInGroupList") {
        for g in groups {
            if let (Some(uid), Some(gname)) = (g.str_val("uuid"), g.str_val("groupName")) {
                group_names.insert(uid.to_string(), gname.to_string());
            }
        }
    }

    let bundle_id = plist
        .str_val("CFBundleIdentifier")
        .map(|s| s.to_string());
    let bundle_ver = plist
        .str_val("CFBundleShortVersionString")
        .or_else(|| plist.str_val("CFBundleVersion"))
        .map(|s| s.to_string());

    for item in plugin_list {
        let Some(uuid) = item.str_val("uuid").filter(|s| !s.is_empty()) else {
            continue;
        };
        let Some(display_name) = item.str_val("displayName").filter(|s| !s.is_empty()) else {
            continue;
        };
        let category = item
            .str_val("group")
            .and_then(|gid| group_names.get(gid))
            .cloned();
        let version = bundle_ver
            .clone()
            .or_else(|| item.str_val("version").map(|s| s.to_string()));
        let class_name = item.str_val("className");
        let normalized_name = normalize_plugin_name(display_name);
        let match_aliases = build_aliases(
            display_name,
            &normalized_name,
            &[class_name],
            bundle_id.as_deref(),
        );

        out.push(InstalledPlugin {
            name: display_name.to_string(),
            normalized_name,
            target_nle: NleFormat::FinalCutPro,
            format: PluginFormat::FxPlug,
            media_kind: TrackKind::Video,
            native_id: format!("FxPlug:{uuid}"),
            bundle_id: bundle_id.clone(),
            category,
            version,
            path: bundle_path.to_string_lossy().into_owned(),
            match_aliases,
        });
    }
}

#[cfg(target_os = "macos")]
fn scan_motion_effects_dir(effects_root: &Path, out: &mut Vec<InstalledPlugin>) {
    let Ok(categories) = fs::read_dir(effects_root) else {
        return;
    };
    for cat_entry in categories.flatten() {
        let cat_path = cat_entry.path();
        if !cat_path.is_dir() {
            continue;
        }
        let cat_name = cat_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if cat_name.starts_with('.') {
            continue;
        }
        let Ok(templates) = fs::read_dir(&cat_path) else {
            continue;
        };
        for tpl_entry in templates.flatten() {
            let tpl_dir = tpl_entry.path();
            if !tpl_dir.is_dir() {
                continue;
            }
            let tpl_name = tpl_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let Ok(files) = fs::read_dir(&tpl_dir) else {
                continue;
            };
            for f in files.flatten() {
                let fp = f.path();
                if fp
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("moef"))
                {
                    let fname = fp
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    let display_name = fp
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&tpl_name)
                        .to_string();
                    let uid = format!("~/Effects.localized/{cat_name}/{tpl_name}/{fname}");
                    let normalized_name = normalize_plugin_name(&display_name);
                    let match_aliases = build_aliases(
                        &display_name,
                        &normalized_name,
                        &[Some(tpl_name.as_str())],
                        None,
                    );
                    out.push(InstalledPlugin {
                        name: display_name,
                        normalized_name,
                        target_nle: NleFormat::FinalCutPro,
                        format: PluginFormat::MotionEffectTemplate,
                        media_kind: TrackKind::Video,
                        native_id: uid,
                        bundle_id: None,
                        category: Some(cat_name.clone()),
                        version: None,
                        path: fp.to_string_lossy().into_owned(),
                        match_aliases,
                    });
                }
            }
        }
    }
}

// ── Cross-NLE Audio Plugin Scanner (AU & VST3) ──────────────────────────

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn scan_audio_plugins_dir(
    dir: &Path,
    expected_ext: &str,
    format: PluginFormat,
    out: &mut Vec<InstalledPlugin>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let host_nles: &[NleFormat] = match format {
        PluginFormat::AudioUnit => &[
            NleFormat::PremierePro,
            NleFormat::DaVinciResolve,
            NleFormat::FinalCutPro,
        ],
        PluginFormat::Vst3 => &[NleFormat::PremierePro, NleFormat::DaVinciResolve],
        _ => &[],
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(expected_ext))
        {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if stem.is_empty()
            || stem.starts_with("Apple")
            || stem.starts_with("ASAF ")
        {
            continue;
        }
        let plist = read_xml_plist_file(&path.join("Contents/Info.plist"));
        let name = plist
            .as_ref()
            .and_then(|p| {
                p.str_val("CFBundleDisplayName")
                    .or_else(|| p.str_val("CFBundleName"))
            })
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| stem.clone());
        let bundle_id = plist
            .as_ref()
            .and_then(|p| p.str_val("CFBundleIdentifier"))
            .map(|s| s.to_string());
        let version = plist
            .as_ref()
            .and_then(|p| {
                p.str_val("CFBundleShortVersionString")
                    .or_else(|| p.str_val("CFBundleVersion"))
            })
            .map(|s| s.to_string());
        let normalized_name = normalize_plugin_name(&name);
        let native_id = bundle_id.clone().unwrap_or_else(|| name.clone());
        let match_aliases = build_aliases(
            &name,
            &normalized_name,
            &[Some(stem.as_str())],
            bundle_id.as_deref(),
        );

        for &nle in host_nles {
            out.push(InstalledPlugin {
                name: name.clone(),
                normalized_name: normalized_name.clone(),
                target_nle: nle,
                format,
                media_kind: TrackKind::Audio,
                native_id: native_id.clone(),
                bundle_id: bundle_id.clone(),
                category: None,
                version: version.clone(),
                path: path.to_string_lossy().into_owned(),
                match_aliases: match_aliases.clone(),
            });
        }
    }
}

// ── Minimal Pure-Rust Apple XML Plist Reader ────────────────────────────

#[derive(Debug, Clone)]
enum XmlPlistVal {
    Str(String),
    Dict(XmlPlistDict),
    Array(Vec<XmlPlistVal>),
}

#[derive(Debug, Clone, Default)]
struct XmlPlistDict {
    entries: BTreeMap<String, XmlPlistVal>,
}

impl XmlPlistDict {
    fn str_val(&self, key: &str) -> Option<&str> {
        match self.entries.get(key) {
            Some(XmlPlistVal::Str(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    fn array_of_dicts(&self, key: &str) -> Option<Vec<&XmlPlistDict>> {
        match self.entries.get(key) {
            Some(XmlPlistVal::Array(arr)) => {
                let dicts: Vec<&XmlPlistDict> = arr
                    .iter()
                    .filter_map(|v| match v {
                        XmlPlistVal::Dict(d) => Some(d),
                        _ => None,
                    })
                    .collect();
                if dicts.is_empty() {
                    None
                } else {
                    Some(dicts)
                }
            }
            _ => None,
        }
    }
}

fn read_xml_plist_file(path: &Path) -> Option<XmlPlistDict> {
    let bytes = fs::read(path).ok()?;
    if bytes.starts_with(b"bplist") {
        // Binary plists are skipped cleanly (Apple bundle Info.plists in MediaCore/OFX/FxPlug are XML)
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    parse_xml_plist(text)
}

fn parse_xml_plist(xml: &str) -> Option<XmlPlistDict> {
    let opts = ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = Document::parse_with_options(xml, opts).ok()?;
    let root_dict = doc
        .root_element()
        .children()
        .find(|n| n.is_element() && n.has_tag_name("dict"))?;
    Some(parse_plist_dict_node(root_dict))
}

fn parse_plist_dict_node(node: Node<'_, '_>) -> XmlPlistDict {
    let mut dict = XmlPlistDict::default();
    let elems: Vec<Node<'_, '_>> = node.children().filter(|n| n.is_element()).collect();
    let mut i = 0;
    while i + 1 < elems.len() {
        if elems[i].has_tag_name("key") {
            let key = elems[i].text().unwrap_or("").trim().to_string();
            if let Some(val) = parse_plist_val_node(elems[i + 1]) {
                dict.entries.insert(key, val);
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    dict
}

fn parse_plist_val_node(node: Node<'_, '_>) -> Option<XmlPlistVal> {
    match node.tag_name().name() {
        "string" | "integer" | "real" => {
            Some(XmlPlistVal::Str(node.text().unwrap_or("").trim().to_string()))
        }
        "dict" => Some(XmlPlistVal::Dict(parse_plist_dict_node(node))),
        "array" => {
            let items = node
                .children()
                .filter(|n| n.is_element())
                .filter_map(parse_plist_val_node)
                .collect();
            Some(XmlPlistVal::Array(items))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_adobe_pipl_8bim_records() {
        // Synthetic Mac PiPL snippet matching real Premiere Pro eFKT .rsrc layout
        let mut buf = vec![0u8; 16];
        let push_prop = |b: &mut Vec<u8>, tag: &[u8; 4], val: &[u8]| {
            b.extend_from_slice(b"8BIM");
            b.extend_from_slice(tag);
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&(val.len() as u32).to_be_bytes());
            b.extend_from_slice(val);
            while !b.len().is_multiple_of(4) {
                b.push(0);
            }
        };
        push_prop(&mut buf, b"kind", b"eFKT");
        push_prop(&mut buf, b"name", b"\x11Redact Anonymizer");
        push_prop(&mut buf, b"catg", b"\x07Aagedal");
        push_prop(&mut buf, b"eMNA", b"\x1bAGDL Multi-Layer Anonymizer");

        let meta = parse_pipl_rsrc(&buf).expect("parse PiPL");
        assert_eq!(meta.kind.as_deref(), Some("eFKT"));
        assert_eq!(meta.name.as_deref(), Some("Redact Anonymizer"));
        assert_eq!(meta.category.as_deref(), Some("Aagedal"));
        assert_eq!(meta.match_name.as_deref(), Some("AGDL Multi-Layer Anonymizer"));
    }

    #[test]
    fn parses_fxplug_info_plist_and_matches_across_all_three_nles() {
        let fxplug_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>me.aagedal.fxplug.anonymizer.service</string>
  <key>CFBundleShortVersionString</key>
  <string>2.0.0</string>
  <key>ProPlugPlugInGroupList</key>
  <array>
    <dict>
      <key>groupName</key><string>Aagedal</string>
      <key>uuid</key><string>83F8AD9D-AC27-40C2-A111-0BFAA17AA9A7</string>
    </dict>
  </array>
  <key>ProPlugPlugInList</key>
  <array>
    <dict>
      <key>className</key><string>AnonymizerFxPlugIn</string>
      <key>displayName</key><string>Redact Anonymizer</string>
      <key>group</key><string>83F8AD9D-AC27-40C2-A111-0BFAA17AA9A7</string>
      <key>uuid</key><string>211035B9-C3CF-42E6-B8CB-F9C28DC3B98A</string>
    </dict>
  </array>
</dict>
</plist>"#;

        let mut list = extract_fxplug_entries_from_plist_xml(
            fxplug_xml,
            Path::new("/Applications/Redact Anonymizer.app/Contents/PlugIns/Redact.pluginkit"),
        );
        assert_eq!(list.len(), 1);
        assert_eq!(
            list[0].native_id,
            "FxPlug:211035B9-C3CF-42E6-B8CB-F9C28DC3B98A"
        );
        assert_eq!(list[0].category.as_deref(), Some("Aagedal"));

        // Add Premiere MediaCore & Resolve OFX counterparts
        list.push(InstalledPlugin {
            name: "Redact Anonymizer".into(),
            normalized_name: normalize_plugin_name("Redact Anonymizer"),
            target_nle: NleFormat::PremierePro,
            format: PluginFormat::PremiereMediaCore,
            media_kind: TrackKind::Video,
            native_id: "AE.AGDL Multi-Layer Anonymizer".into(),
            bundle_id: Some("me.aagedal.premiere.anonymizer".into()),
            category: Some("Aagedal".into()),
            version: Some("2.0.0".into()),
            path: "/Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore/Anonymizer.plugin".into(),
            match_aliases: build_aliases(
                "Redact Anonymizer",
                "redact anonymizer",
                &[Some("AGDL Multi-Layer Anonymizer"), Some("Anonymizer")],
                Some("me.aagedal.premiere.anonymizer"),
            ),
        });
        list.push(InstalledPlugin {
            name: "Redact Anonymizer".into(),
            normalized_name: normalize_plugin_name("Redact Anonymizer OFX"),
            target_nle: NleFormat::DaVinciResolve,
            format: PluginFormat::OpenFx,
            media_kind: TrackKind::Video,
            native_id: "me.aagedal.ofx.MultiLayerAnonymizer".into(),
            bundle_id: Some("me.aagedal.ofx.MultiLayerAnonymizer".into()),
            category: None,
            version: Some("2.0.0".into()),
            path: "/Library/OFX/Plugins/Anonymizer.ofx.bundle".into(),
            match_aliases: build_aliases(
                "Redact Anonymizer",
                "redact anonymizer",
                &[Some("Redact Anonymizer OFX"), Some("Anonymizer")],
                Some("me.aagedal.ofx.MultiLayerAnonymizer"),
            ),
        });

        let reg = InstalledPluginRegistry::from_plugins(list);
        let groups = reg.grouped_plugins(2);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].installations.len(), 3);

        // Premiere -> Resolve OFX
        let to_resolve = reg.find_target_matches(
            NleFormat::PremierePro,
            NleFormat::DaVinciResolve,
            "AE.AGDL Multi-Layer Anonymizer",
            "Redact Anonymizer",
            TrackKind::Video,
        );
        assert_eq!(to_resolve.len(), 1);
        assert_eq!(to_resolve[0].native_id, "me.aagedal.ofx.MultiLayerAnonymizer");

        // Premiere -> Final Cut Pro FxPlug
        let to_fcp = reg.find_target_matches(
            NleFormat::PremierePro,
            NleFormat::FinalCutPro,
            "AE.AGDL Multi-Layer Anonymizer",
            "Redact Anonymizer",
            TrackKind::Video,
        );
        assert_eq!(to_fcp.len(), 1);
        assert_eq!(
            to_fcp[0].native_id,
            "FxPlug:211035B9-C3CF-42E6-B8CB-F9C28DC3B98A"
        );

        // Final Cut Pro Motion Effect template -> Premiere MediaCore
        let fcp_to_pr = reg.find_target_matches(
            NleFormat::FinalCutPro,
            NleFormat::PremierePro,
            "fcp.Redact Anonymizer:~/Effects.localized/Aagedal/Redact Anonymizer/Redact Anonymizer.moef",
            "Redact Anonymizer",
            TrackKind::Video,
        );
        assert_eq!(fcp_to_pr.len(), 1);
        assert_eq!(fcp_to_pr[0].native_id, "AE.AGDL Multi-Layer Anonymizer");
    }
}
