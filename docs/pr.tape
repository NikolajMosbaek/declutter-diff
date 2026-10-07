# Records docs/images/pr.gif:  vhs docs/pr.tape   (from the repository root)
# The pull request is pretend: `cargo run --example demo -- pr` serves it from a local
# repository and replaces `gh` with a stub, so nothing is fetched from or posted to GitHub.

Output docs/images/pr.gif

Set Shell "bash"
Set FontSize 15
Set Width 1360
Set Height 760
Set Padding 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 60ms
Set CursorBlink false
Env COLORTERM "truecolor"

Hide
Type "cargo build --release --quiet && cargo run --quiet --example demo -- pr target/demo-pr" Enter
Wait+Screen@180s /demo pull request ready/
Type "export PATH=$PWD/target/demo-pr/bin:$PWD/target/release:$PATH PS1='$ ' && cd target/demo-pr/shop && clear" Enter
Sleep 500ms
Show

# Paste the PR's URL from the browser.
Type "declutter https://github.com/acme/shop/pull/42" Sleep 500ms Enter
Sleep 1.5s
Down Sleep 2.5s

# Find the discount check and leave a note on it.
Type "/" Sleep 300ms
Type "discount <" Sleep 400ms
Enter Sleep 1.2s
Type "m" Sleep 400ms
Type "Should a 100% discount be allowed?" Sleep 400ms
Enter Sleep 2s

# Quit: declutter offers to post the note to the PR.
Type "q" Sleep 2.5s
Type "y" Sleep 500ms
Enter Sleep 3s
