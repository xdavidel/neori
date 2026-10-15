# neori-visual-tests

## What is this?

This is a visual testing tool for neori.

It uses the real neori layout and rendering code, but with mock windows instead of Wayland clients.
The idea is to go through the test scenarios and check that everything *looks* right.

## Running

You will need recent GTK and libadwaita.
Then, `cargo run`.
