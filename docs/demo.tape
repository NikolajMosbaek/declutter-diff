# Records docs/images/demo.gif:  vhs docs/demo.tape   (from the repository root)
# Needs vhs (brew install vhs). The demo change comes from examples/demo.rs.

Output docs/images/demo.gif

Set Shell "bash"
Set FontSize 15
Set Width 1360
Set Height 760
Set Padding 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 75ms
Set CursorBlink false
Env COLORTERM "truecolor"

Hide
Type "cargo build --release --quiet && cargo run --quiet --example demo -- repo target/demo-repo" Enter
Wait+Screen@180s /demo repository ready/
Type "export PATH=$PWD/target/release:$PATH PS1='$ ' && cd target/demo-repo && clear" Enter
Sleep 500ms
Show

# On a branch, plain `declutter` reviews it against main, with comments hidden.
Type "declutter" Sleep 400ms Enter
Sleep 1.5s
Down Sleep 3s

# Show the comments the change added, then hide them again.
Type "c" Sleep 3s
Type "c" Sleep 1.5s

# Only the comments: what the AI says its code does.
Type "C" Sleep 3s
Type "C" Sleep 1s

# Search for the discount check — the cursor lands on it — and leave a note there.
Type "/" Sleep 300ms
Type "discount <" Sleep 400ms
Enter Sleep 1.5s
Type "m" Sleep 500ms
Type "Should a 100% discount be allowed?" Sleep 500ms
Enter Sleep 2s
Escape Sleep 200ms
Escape Sleep 800ms

# ] jumps to the next change, here into format.ts; f hides its formatting-only change.
Type "]" Sleep 2s
Type "f" Sleep 2.5s
Type "f" Sleep 1s

# Only the tests, then everything again.
Type "T" Sleep 2.5s
Type "T" Sleep 1s

# Every key.
Type "?" Sleep 3.5s
Type " " Sleep 500ms
Type "q" Sleep 1s
