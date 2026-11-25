// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The browser debug relay (an agent debugging a `loft --html --debug` client through a game
//! server) drove the Rust writer's page and retired with the writer (@PLN225 R4); a page built
//! through the C has no debug channel, and `--html --debug` is refused at the build —
//! `tests/html_wasm.rs::html_debug_is_refused`.
