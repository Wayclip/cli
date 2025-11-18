use anyhow::{Result, bail};

pub fn sanitize_and_validate_filename_stem(new_name_input: &str) -> Result<String> {
    let trimmed = new_name_input.trim();
    if trimmed.is_empty() {
        bail!("New name cannot be empty.");
    }
    let sanitized = trimmed.replace(' ', "_");

    if sanitized
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
    {
        bail!("New name contains invalid characters (< > : \" / \\ | ? *).");
    }
    Ok(sanitized)
}

pub fn validate_ffmpeg_time(time_str: &str) -> Result<String> {
    let trimmed = time_str.trim();
    if trimmed.parse::<f64>().is_ok() {
        return Ok(trimmed.to_string());
    }
    let parts: Vec<&str> = trimmed.split(':').collect();
    if parts.len() > 3 || parts.is_empty() {
        bail!("Invalid time format '{time_str}'. Use seconds (e.g., 5.5) or HH:MM:SS format.",);
    }
    if parts
        .iter()
        .all(|p| !p.is_empty() && p.parse::<f64>().is_ok())
    {
        Ok(trimmed.to_string())
    } else {
        bail!("Invalid time format '{time_str}'. Use seconds (e.g., 5.5) or HH:MM:SS format.",);
    }
}
