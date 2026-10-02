//! The former ordinary-login-to-proof diagnostic is intentionally retired.
//! Use the official Browser CLI to approve and exercise recording storage.
fn main() {
    eprintln!(
        "The raw OBO upload diagnostic is retired. Use browser recording-access start, then browser recording-access complete <authorization-id> --code-file <private-file> --state <state>. Start and end a test browser session to exercise reservation, capability transfer and commit."
    );
    std::process::exit(2);
}
