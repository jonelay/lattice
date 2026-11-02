//! Fenced-code tracking for Markdown adapters, so every adapter that reads
//! Markdown agrees about which lines are code.

/// The fence character, run length and info string of a line that could open a
/// fence: any indentation, then three or more backticks or tildes. Any
/// indentation, because a fence continuing a list item sits four or more columns
/// in. As CommonMark has it, a backtick run whose info string holds a backtick is
/// inline code, not a fence; a tilde fence's info string may hold either.
fn fence_run(line: &str) -> Option<(char, usize, &str)> {
    let rest = line.trim_start_matches([' ', '\t']);
    let fence_char = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = rest.chars().take_while(|c| *c == fence_char).count();
    // Both fence characters are one byte, so the run length is a byte offset.
    let info = &rest[run..];
    (run >= 3 && !(fence_char == '`' && info.contains('`'))).then_some((fence_char, run, info))
}

/// Line-by-line fenced-code state. Feed every line of a file, in order, to
/// [`FenceState::in_code`]; skipping one desynchronizes the state.
///
/// ```
/// use latt_adapter::fence::FenceState;
///
/// let mut fences = FenceState::new();
/// let lines = ["text", "```md", "## not a heading", "```", "## heading"];
/// let code: Vec<bool> = (1..).zip(lines).map(|(n, l)| fences.in_code(l, n)).collect();
/// assert_eq!(code, [false, true, true, true, false]);
/// assert_eq!(fences.unclosed(), None);
/// ```
#[derive(Debug, Default)]
pub struct FenceState {
    open: Option<(char, usize)>,
    opened_at: u32,
}

impl FenceState {
    /// State at the start of a file: no fence open.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// True when the line is a fence or lies inside one. A fence closes only on a
    /// run of the opening character at least as long, followed by nothing but
    /// whitespace.
    ///
    /// A fence opens at any indentation, not CommonMark's zero to three spaces,
    /// because a fence continuing a list item sits four or more columns in.
    /// `line_num` is the line's number in the file; the one that opens a fence
    /// is what [`unclosed`](Self::unclosed) reports.
    pub fn in_code(&mut self, line: &str, line_num: u32) -> bool {
        if let Some((fence_char, fence_len)) = self.open {
            if let Some((run_char, run_len, rest)) = fence_run(line)
                && run_char == fence_char
                && run_len >= fence_len
                && rest.trim().is_empty()
            {
                self.open = None;
            }
            return true;
        }
        let Some((fence_char, run_len, _)) = fence_run(line) else {
            return false;
        };
        self.open = Some((fence_char, run_len));
        self.opened_at = line_num;
        true
    }

    /// The line of a fence that is still open, once every line has been seen:
    /// everything after it was read as code, which the caller must report.
    #[must_use]
    pub fn unclosed(&self) -> Option<u32> {
        self.open.map(|_| self.opened_at)
    }
}

#[cfg(test)]
mod tests {
    use super::FenceState;

    fn code_lines(text: &str) -> (Vec<bool>, Option<u32>) {
        let mut fences = FenceState::new();
        let code = (1..)
            .zip(text.lines())
            .map(|(n, l)| fences.in_code(l, n))
            .collect();
        (code, fences.unclosed())
    }

    #[test]
    fn a_shorter_or_other_character_run_does_not_close() {
        let (code, unclosed) = code_lines("````\n```\n~~~~\n````\nafter");
        assert_eq!(code, [true, true, true, true, false]);
        assert_eq!(unclosed, None);
    }

    #[test]
    fn a_closer_with_trailing_text_does_not_close() {
        let (_, unclosed) = code_lines("```\n``` not a closer\n");
        assert_eq!(unclosed, Some(1));
    }

    #[test]
    fn an_indented_fence_opens() {
        let (code, _) = code_lines("- item\n    ```\n    ## x\n    ```\n");
        assert_eq!(code, [false, true, true, true]);
    }

    #[test]
    fn a_backtick_in_a_backtick_info_string_is_inline_code_not_a_fence() {
        let (code, unclosed) = code_lines("```a`b```\n## heading\n");
        assert_eq!(code, [false, false]);
        assert_eq!(unclosed, None);
    }

    #[test]
    fn a_tilde_fence_info_string_may_hold_either_fence_character() {
        let (code, unclosed) = code_lines("~~~ a`b~c\n## x\n~~~\nafter\n");
        assert_eq!(code, [true, true, true, false]);
        assert_eq!(unclosed, None);
    }

    #[test]
    fn two_backticks_are_inline_code_not_a_fence() {
        let (code, unclosed) = code_lines("``code``\n");
        assert_eq!(code, [false]);
        assert_eq!(unclosed, None);
    }

    #[test]
    fn a_closer_with_trailing_whitespace_closes() {
        let (code, unclosed) = code_lines("```\n```  \t\nafter\n");
        assert_eq!(code, [true, true, false]);
        assert_eq!(unclosed, None);
    }

    mod properties {
        use proptest::prelude::*;

        use super::FenceState;

        fn fence_char() -> impl Strategy<Value = char> {
            prop_oneof![Just('`'), Just('~')]
        }

        /// Text after a run: holds no fence character, so it cannot extend the run
        /// or make a backtick run inline code.
        fn suffix() -> impl Strategy<Value = String> {
            "[a-z \t]{0,4}"
        }

        proptest! {
            /// The law: an open fence closes on exactly a run of its own character, at
            /// least as long as the opener, followed by whitespace only; at any
            /// indentation of either line. Every line from the opener on is code, and
            /// a fence left open is reported at the opener's line.
            #[test]
            fn closes_only_on_a_matching_run(
                open_char in fence_char(), open_len in 3usize..7, open_indent in 0usize..6,
                info in suffix(),
                close_char in fence_char(), close_len in 1usize..9, close_indent in 0usize..6,
                trailing in suffix(),
                open_line in 1u32..50,
            ) {
                let opener = format!(
                    "{}{}{info}",
                    " ".repeat(open_indent),
                    open_char.to_string().repeat(open_len),
                );
                let closer = format!(
                    "{}{}{trailing}",
                    " ".repeat(close_indent),
                    close_char.to_string().repeat(close_len),
                );
                let closes = close_char == open_char
                    && close_len >= open_len
                    && trailing.trim().is_empty();

                let mut fences = FenceState::new();
                prop_assert!(fences.in_code(&opener, open_line));
                prop_assert!(fences.in_code(&closer, open_line + 1));
                prop_assert_eq!(fences.unclosed(), (!closes).then_some(open_line));
                prop_assert_eq!(fences.in_code("after", open_line + 2), !closes);
            }
        }
    }
}
