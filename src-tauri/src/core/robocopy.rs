pub fn robocopy_exit_ok(code: i32) -> bool {
    code < 8
}

pub fn robocopy_exit_description(code: i32) -> &'static str {
    match code {
        0 => "No files copied. No failure.",
        1 => "Files copied successfully.",
        2 => "Extra files or directories detected.",
        3 => "Files copied and extra files detected.",
        4 => "Mismatched files or directories detected.",
        5 => "Files copied and mismatches detected.",
        6 => "Extra files and mismatches detected.",
        7 => "Files copied, extra files, and mismatches detected.",
        _ => "Robocopy reported a fatal error.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robocopy_codes_under_eight_are_successful() {
        for code in 0..8 {
            assert!(robocopy_exit_ok(code));
        }
        assert!(!robocopy_exit_ok(8));
        assert!(!robocopy_exit_ok(16));
    }
}
