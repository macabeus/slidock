//! The one style rule a document may hold.

/// The reset rule, byte for byte, in a `<style>` in a document's head; everything else is inline. The lint refuses
/// any other rule.
pub const RESET_STYLE: &str = "html,body{margin:0;padding:0}p{margin:0}";

#[cfg(test)]
mod tests {
    #[test]
    fn keeps_one_fixed_reset_rule() {
        assert_eq!(
            super::RESET_STYLE,
            "html,body{margin:0;padding:0}p{margin:0}"
        );
    }
}
