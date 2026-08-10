// @nexus-sheet/adapter-sdk/testing — the one-line contract test harness (§16).
//
//   describe("roll20", () => runContractTests(adapter, fixtures));
//
// Asserts import(export(fixture)) round-trips within fieldCoverage; validate()
// rejects known-bad input; detectChanges is a no-op on unchanged input; unknown
// content produces warnings rather than throwing (§5.2). Runs standalone in
// pure TS — no Rust, no host, no DB, no network (§16, §21).
//
// Phase 0.1: scaffold only. Lands in 1.2.
export {};
