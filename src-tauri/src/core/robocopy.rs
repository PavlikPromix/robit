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

pub fn robocopy_line_indicates_access_block(line: &str) -> bool {
    let upper = line.to_ascii_uppercase();
    upper.contains("ERROR 5 ")
        || upper.contains("ERROR 5 (")
        || upper.contains("ERROR 32 ")
        || upper.contains("ERROR 32 (")
        || upper.contains("ACCESS IS DENIED")
        || upper.contains("BEING USED BY ANOTHER PROCESS")
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

    #[test]
    fn detects_access_denied_and_in_use_errors() {
        assert!(robocopy_line_indicates_access_block(
            "2026/06/14 01:46:45 ERROR 5 (0x00000005) Deleting Source File C:\\x.dll"
        ));
        assert!(robocopy_line_indicates_access_block("Access is denied."));
        assert!(robocopy_line_indicates_access_block(
            "The process cannot access the file because it is being used by another process."
        ));
        assert!(!robocopy_line_indicates_access_block(
            "Files copied successfully."
        ));
    }
}
