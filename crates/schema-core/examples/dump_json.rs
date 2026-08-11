//! Debug helper: dumps a minimal `Character` as JSON. Used to generate fixture
//! data for cross-language round-trip tests (§16). Not part of the public API.

use schema_core::testing::minimal_build;

fn main() {
    let build = minimal_build();
    let character = serde_json::json!({
        "build": build,
        "meta": {
            "id": "01HXYZ1234567890ABCDEFGHJK",
            "displayName": "Test Character",
            "ownerId": "01HXYZ1234567890ABCDEFGHJK",
            "system": "pf2e",
            "schemaVersion": "0.1.0",
            "contentHash": "0000000000000000000000000000000000000000000000000000000000000000",
            "createdAt": "2025-01-01T00:00:00Z",
            "updatedAt": "2025-01-01T00:00:00Z"
        },
        "session": {
            "platform": "foundry",
            "hitPoints": { "current": 10, "temporary": 0, "max": 10 },
            "spellSlots": {},
            "focusPoints": { "current": 0, "max": 0 },
            "heroPoints": 0,
            "conditions": [],
            "temporaryModifiers": [],
            "encounterEffects": [],
            "lastObservedAt": "2025-01-01T00:00:00Z"
        },
        "extensions": {}
    });
    println!("{}", serde_json::to_string_pretty(&character).unwrap());
}
