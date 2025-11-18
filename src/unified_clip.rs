use anyhow::{Context, Result, bail};
use wayclip_core::gather_unified_clips;
use wayclip_core::models::UnifiedClipData;

pub async fn find_unified_clip(name_input: &str) -> Result<UnifiedClipData> {
    let trimmed_name = name_input.trim();

    let name_stem = if trimmed_name.to_lowercase().ends_with(".mp4") {
        &trimmed_name[..trimmed_name.len() - 4]
    } else {
        trimmed_name
    };

    if name_stem.is_empty() {
        bail!("Clip name cannot be empty.");
    }

    let all_clips = gather_unified_clips().await?;
    all_clips
        .into_iter()
        .find(|clip| clip.name.eq_ignore_ascii_case(name_stem))
        .context(format!("Clip '{name_stem}' not found."))
}
