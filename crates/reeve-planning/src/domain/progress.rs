/// completed / (total - dropped); None when nothing counts.
pub fn progress(completed: u32, dropped: u32, total: u32) -> Option<f32> {
    let denom = total.saturating_sub(dropped);
    if denom == 0 {
        None
    } else {
        Some(completed as f32 / denom as f32)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excludes_dropped_and_hides_empty() {
        assert_eq!(progress(0, 0, 0), None);
        assert_eq!(
            progress(0, 3, 3),
            None,
            "an initiative whose stories were all dropped shows nothing, not 100%"
        );
        assert_eq!(progress(2, 1, 5), Some(0.5));
    }
}
