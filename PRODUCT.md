# Product

<!-- impeccable:product-schema 1 -->

## Platform

adaptive

## Stack

delegated: Rust desktop application with an egui prototype and a GTK3/WebKitGTK Linux backend

## Users

Inferred from the project brief: people who want a desktop browser with privacy
and security controls that are understandable and reversible.

## Product Purpose

Inferred from the project brief: provide web navigation with conservative
security defaults, visible decisions, and no telemetry by default.

## Capabilities and Constraints

- Linux is the first supported desktop platform.
- HTTPS is preferred and HTTP exceptions are explicit.
- The WebKitGTK context is ephemeral in the current backend.
- Browser settings must affect the shared shell policy, not only the UI.
- Extensions and complete site partitioning remain outside the current MVP.

## Product Principles

- Privacy-preserving defaults.
- Security decisions are visible and reversible.
- Small, testable policy boundaries.
- No silent expansion of network, disk, or execution privileges.

## Accessibility & Inclusion

The settings surface should use native controls, explicit labels, keyboard
focus, and explanatory text instead of relying on color alone.
