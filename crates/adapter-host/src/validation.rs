//! Boundary validation — adapter output is untrusted and validated here, in
//! Rust, against the canonical schema before the engine ever sees it (§5.5, §1).
//!
//! The check is structural: `serde_json::from_value` with `deny_unknown_fields`
//! is total — if deserialization succeeds, the data is structurally valid by
//! construction. Present fields are type-checked; unknown fields are rejected.
//! This is the security boundary that makes adapters untrusted (§1).

use crate::error::HostError;
use crate::outcome::{ChangeSet, ExportOutcome, ImportOutcome, ValidationReport};

/// Validate an `ImportOutcome` returned by the adapter.
///
/// Deserialization into `ImportOutcome` is the boundary check: the `build`
/// field is a `PartialCharacterBuild` with `deny_unknown_fields`, so unknown
/// field names are rejected. Present fields are type-checked by serde (§5.5).
pub fn validate_import(raw: serde_json::Value) -> Result<ImportOutcome, HostError> {
    let outcome: ImportOutcome = serde_json::from_value(raw)
        .map_err(|e| HostError::BoundaryValidation(format!("import outcome: {e}")))?;
    Ok(outcome)
}

/// Validate an `ExportOutcome` returned by the adapter.
pub fn validate_export(raw: serde_json::Value) -> Result<ExportOutcome, HostError> {
    let outcome: ExportOutcome = serde_json::from_value(raw)
        .map_err(|e| HostError::BoundaryValidation(format!("export outcome: {e}")))?;
    Ok(outcome)
}

/// Validate a `ChangeSet` returned by `detectChanges`.
pub fn validate_changes(raw: serde_json::Value) -> Result<ChangeSet, HostError> {
    let changes: ChangeSet = serde_json::from_value(raw)
        .map_err(|e| HostError::BoundaryValidation(format!("change set: {e}")))?;
    Ok(changes)
}

/// Validate a `ValidationReport` returned by `validate`.
pub fn validate_report(raw: serde_json::Value) -> Result<ValidationReport, HostError> {
    let report: ValidationReport = serde_json::from_value(raw)
        .map_err(|e| HostError::BoundaryValidation(format!("validation report: {e}")))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_import_passes() {
        let raw = serde_json::json!({
            "build": {
                "identity": { "name": "Aria" }
            },
            "warnings": [],
            "unmapped": []
        });
        assert!(validate_import(raw).is_ok());
    }

    #[test]
    fn unknown_field_in_build_rejected() {
        let raw = serde_json::json!({
            "build": {
                "identity": { "name": "Aria" },
                "totallyNotAField": true
            },
            "warnings": []
        });
        let err = validate_import(raw).unwrap_err();
        assert!(matches!(err, HostError::BoundaryValidation(_)));
    }

    #[test]
    fn unknown_field_in_outcome_rejected() {
        let raw = serde_json::json!({
            "build": {},
            "warnings": [],
            "unmapped": [],
            "extraField": true
        });
        assert!(validate_import(raw).is_err());
    }

    #[test]
    fn wrong_type_rejected() {
        let raw = serde_json::json!({
            "build": {
                "level": "twenty"
            },
            "warnings": []
        });
        assert!(validate_import(raw).is_err());
    }

    #[test]
    fn empty_build_accepted() {
        let raw = serde_json::json!({
            "build": {},
            "warnings": [],
            "unmapped": []
        });
        let outcome = validate_import(raw).unwrap();
        assert!(outcome.build.is_empty());
    }

    #[test]
    fn warnings_with_unknown_field_rejected() {
        let raw = serde_json::json!({
            "build": {},
            "warnings": [{
                "path": "build.feats[0]",
                "kind": { "type": "unmappedField" },
                "detail": "Unknown feat",
                "extra": true
            }],
            "unmapped": []
        });
        assert!(validate_import(raw).is_err());
    }

    #[test]
    fn fuzzy_match_warning_accepted() {
        let raw = serde_json::json!({
            "build": {},
            "warnings": [{
                "path": "build.feats[0]",
                "kind": { "type": "fuzzyMatch", "confidence": 0.85 },
                "detail": "Matched 'Power Attack' to 'power-attack'"
            }],
            "unmapped": []
        });
        assert!(validate_import(raw).is_ok());
    }

    #[test]
    fn export_outcome_validated() {
        let raw = serde_json::json!({
            "payload": { "type": "character", "name": "Aria" },
            "warnings": [],
            "skipped": ["build.drones"]
        });
        assert!(validate_export(raw).is_ok());
    }

    #[test]
    fn change_set_validated() {
        let raw = serde_json::json!({
            "hasChanges": true,
            "changedPaths": ["build.level"]
        });
        let cs = validate_changes(raw).unwrap();
        assert!(cs.has_changes);
    }
}
