# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/bearcove/hotmeal/releases/tag/hotmeal-v0.1.0) - 2026-01-22

### Added

- integrate cinereus tree diffing directly into hotmeal ([#2](https://github.com/bearcove/hotmeal/pull/2))
- switch diff/patch machinery to untyped DOM (Element/Content)
- add WASM bindings and fix apply.rs for all FlowContent variants
- *(hotmeal)* add HTML serializer
- add diff module for HTML diffing and patch application
- initial hotmeal crate with html5ever parser and typed DOM

### Fixed

- improve parser safety and serialization correctness
- address clippy warnings in untyped_dom
- configure fuzzing workspace correctly

### Other

- Remove fuzz corpus files (they're gitignored)
- replace typed DOM with simpler untyped DOM
- add comprehensive test suite

## [0.42.1](https://github.com/bearcove/hotmeal/compare/cinereus-v0.42.0...cinereus-v0.42.1) - 2026-01-22

### Added

- integrate cinereus tree diffing directly into hotmeal ([#2](https://github.com/bearcove/hotmeal/pull/2))
