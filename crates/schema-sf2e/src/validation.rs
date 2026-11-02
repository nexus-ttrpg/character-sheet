//! SF2e semantic validation (§3.1, §10.2).
//!
//! This is the second validation layer. `serde` + `deny_unknown_fields` already
//! enforce structural validity by construction (if you have a `CharacterBuild`,
//! it deserialized cleanly). This module enforces cross-field and ruleset rules
//! the type system can't express.
//!
//! SF2e shares its validation chassis with PF2e (§3.4); the shared rules live
//! in [`schema_core::chassis`]. SF2e-specific rules — drone chassis/mod
//! constraints, currency sanity, vehicle role checks — are layered on top.
//!
//! Philosophy (§5.2): unknown/homebrew content is a **warning**, never an error.

use schema_core::character::CharacterBuild;
use schema_core::chassis;
use schema_core::newtypes::FieldPath;
use schema_core::ValidationReport;

/// Validate an SF2e `CharacterBuild` (§10.2). Delegates to the shared chassis
/// for cross-system rules, then layers SF2e-specific checks on top.
pub fn validate(build: &CharacterBuild) -> ValidationReport {
    let mut report = chassis::validate_chassis(build);
    validate_drones(build, &mut report);
    validate_currencies(build, &mut report);
    validate_vehicles(build, &mut report);
    report
}

/// SF2e drone validation (§3.4).
///
/// - A drone without a chassis is a warning (not an error — homebrew/unkeyed
///   drones are allowed, §5.2).
/// - Mods with empty slugs are an error (structural, not semantic).
/// - More than 5 mods on a single drone is a warning (the typical SF2e cap
///   scales with level, but 5 is a reasonable static upper bound for a
///   validation hint).
fn validate_drones(build: &CharacterBuild, report: &mut ValidationReport) {
    for (i, drone) in build.drones.iter().enumerate() {
        let base = format!("build.drones[{i}]");

        if drone.chassis.is_none() {
            report.push_warning(schema_core::ValidationWarning {
                path: FieldPath::new(format!("{base}.chassis")),
                message: "drone has no chassis type — homebrew or unkeyed drone".into(),
            });
        }

        for (j, m) in drone.mods.iter().enumerate() {
            if m.slug.as_str().is_empty() {
                report.push_error(schema_core::ValidationError {
                    path: FieldPath::new(format!("{base}.mods[{j}].slug")),
                    message: "drone mod slug must not be empty".into(),
                });
            }
        }

        if drone.mods.len() > 5 {
            report.push_warning(schema_core::ValidationWarning {
                path: FieldPath::new(format!("{base}.mods")),
                message: format!(
                    "drone has {} mods — verify against the character's level-based mod cap",
                    drone.mods.len()
                ),
            });
        }
    }
}

/// SF2e currency sanity (§3.4).
///
/// SF2e uses credits and UPB (Universal Point Base) instead of gp/sp/cp/pp.
/// A build carrying both PF2e and SF2e currencies simultaneously isn't an
/// error (crossover campaigns exist, §3.4) but warrants a warning so the
/// player can confirm intent.
fn validate_currencies(build: &CharacterBuild, report: &mut ValidationReport) {
    let has_pf2e = build.currencies.gp.is_some()
        || build.currencies.sp.is_some()
        || build.currencies.cp.is_some()
        || build.currencies.pp.is_some();
    let has_sf2e = build.currencies.credits.is_some() || build.currencies.upb.is_some();

    if has_pf2e && has_sf2e {
        report.push_warning(schema_core::ValidationWarning {
            path: FieldPath::new("build.currencies"),
            message: "build carries both PF2e (gp/sp/cp/pp) and SF2e (credits/UPB) \
                      currencies — confirm this is a crossover campaign"
                .into(),
        });
    }
}

/// SF2e vehicle validation (§3.4).
///
/// A vehicle without a role is a warning (the role field is optional and may
/// be unset for vehicles the character owns but isn't currently piloting).
fn validate_vehicles(build: &CharacterBuild, report: &mut ValidationReport) {
    for (i, vehicle) in build.vehicles.iter().enumerate() {
        if vehicle.role.is_none() {
            report.push_warning(schema_core::ValidationWarning {
                path: FieldPath::new(format!("build.vehicles[{i}].role")),
                message: "vehicle has no role — character owns but may not be piloting".into(),
            });
        }
    }
}
