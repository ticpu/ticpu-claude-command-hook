use crate::output::HookOutput;

const SYSTEM: &str = "design-rationale.md edited — stopping for review.";

const CONTEXT: &str = "Edited a design-rationale.md. STOP: present the diff and wait for the user to \
review it. No further edits, no commit, no proceeding to code until they approve. Write only sourced \
rationale — never fabricate a reason.";

pub fn check(file_path: &str) -> Option<HookOutput> {
    file_path
        .ends_with("design-rationale.md")
        .then(|| HookOutput::context(SYSTEM, CONTEXT))
}

#[cfg(test)]
mod tests {
    use super::check;

    #[test]
    fn fires_on_design_rationale() {
        assert!(check("docs/design-rationale.md").is_some());
        assert!(check("/abs/path/crate/docs/design-rationale.md").is_some());
    }

    #[test]
    fn silent_otherwise() {
        assert!(check("src/main.rs").is_none());
        assert!(check("README.md").is_none());
        assert!(check("").is_none());
    }
}
