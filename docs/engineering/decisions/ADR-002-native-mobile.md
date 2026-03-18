# ADR-002: Native Swift and Kotlin for Mobile

## Status
Accepted

## Context
Mobile platforms require deep access to Bluetooth Low Energy, WiFi Direct, background
execution APIs, and platform-specific entitlements. The question was whether to use
a cross-platform mobile framework or write native code for iOS and Android separately.

Options considered:

**Option A — React Native**
Large ecosystem, JavaScript/TypeScript, single codebase for both platforms. However,
BLE and WiFi Direct APIs on mobile are deeply platform-specific and poorly served by
React Native's bridge model. Background execution behavior — the hardest problem in
this entire stack — requires platform-specific entitlements and lifecycle management
that are difficult to reason about through an abstraction layer. Performance overhead
of the JS bridge is also a concern for a networking-intensive application.

**Option B — Flutter**
Better performance than React Native, single codebase, strong plugin ecosystem.
Same fundamental problem: the transport layer APIs we need are platform-specific
and the abstraction layer obscures exactly the behavior we need to control precisely.
Dart is also an additional language investment with less payoff than Rust.

**Option C — Kotlin Multiplatform (KMP)**
Share business logic across iOS and Android while writing native UI per platform.
Full native API access. Maturing rapidly but still has rough edges, and the shared
logic layer is less relevant here since the Rust core already handles shared logic.

**Option D — Native Swift (iOS) + Native Kotlin (Android)**
Full platform control. Direct access to CoreBluetooth, Multipeer Connectivity,
WiFi Direct, Android foreground services, background Bluetooth modes, and all
platform-specific entitlements. No abstraction layer between our code and the
platform APIs we depend on most.

## Decision
Option D — native Swift for iOS and native Kotlin for Android.

The two-codebase cost is real but manageable because the Rust core handles all
shared logic. The native layer is responsible only for platform-specific concerns:
BLE scanning and advertising, WiFi Direct / Multipeer session management, GPS,
background service lifecycle, and UI. This is a well-defined and relatively
stable surface area.

The platform-specific code that must be native is exactly the code where having
full platform control matters most. Background execution on iOS in particular
requires working directly with Apple's background mode entitlements, BGAppRefreshTask,
and Multipeer Connectivity — all of which behave differently than documented when
accessed through an abstraction layer.

## Consequences

**Positive:**
- Full access to all platform BLE, WiFi, and background APIs
- No bridge overhead or abstraction layer obscuring platform behavior
- Background execution can be tuned precisely per platform
- Easier to apply for Apple's background Bluetooth entitlements with native code
- Platform-specific optimizations (battery, memory) are directly accessible

**Negative:**
- Two separate mobile codebases to maintain
- iOS and Android developers need different skill sets
- Feature parity must be deliberately maintained across platforms
- iOS remains constrained by Apple's background execution policies regardless
  of implementation approach — native code doesn't eliminate this problem,
  it just gives us the best possible tools to work within it
