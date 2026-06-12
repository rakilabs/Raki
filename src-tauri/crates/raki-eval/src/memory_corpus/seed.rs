//! Hand-authored synthetic real-notes corpus for R4 measurement.
//! All content is fictional and anonymized.

use raki_domain::{Note, NoteId};

pub struct QueryCase {
    pub query: &'static str,
    pub expected_note_ids: &'static [&'static str],
    pub rationale: &'static str,
}

/// Returns a deterministic, anonymized corpus of ~25 notes spanning projects,
/// people, trips, finances, health, ideas, recipes, and books.
/// Access-pattern comments describe how each note is expected to behave in
/// evaluation scenarios (pinned, frequently reopened, old and untouched, etc.).
pub fn seed_notes() -> Vec<Note> {
    vec![
        // === TRIPS (intentionally similar titles with different years) ===
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000001").unwrap(),
            title: "Japan Trip 2025".to_string(),
            body: raki_domain::text_to_body(
                "Planning budget for Japan trip 2025. Tokyo, Kyoto, Osaka. Ryokan cash tips.",
            ),
            created_at: 1_000_000_000_000,
            updated_at: 1_050_000_000_000, // frequently reopened
            deleted_at: None,
            version: 3,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000002").unwrap(),
            title: "Japan Trip 2023".to_string(),
            body: raki_domain::text_to_body("Old Japan trip notes from 2023. Osaka food tour."),
            created_at: 900_000_000_000,
            updated_at: 900_000_000_000, // old and untouched
            deleted_at: None,
            version: 1,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000004").unwrap(),
            title: "Italy Trip 2024".to_string(),
            body: raki_domain::text_to_body(
                "Rome and Florence itinerary. Train schedules, museum passes, gelato notes.",
            ),
            created_at: 970_000_000_000,
            updated_at: 980_000_000_000,
            deleted_at: None,
            version: 2,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000005").unwrap(),
            title: "Spain Trip 2026".to_string(),
            body: raki_domain::text_to_body(
                "Early ideas for Spain 2026. Barcelona architecture and Seville flamenco.",
            ),
            created_at: 1_030_000_000_000,
            updated_at: 1_040_000_000_000,
            deleted_at: None,
            version: 1,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000006").unwrap(),
            title: "Trip Packing List".to_string(),
            body: raki_domain::text_to_body(
                "Reusable packing checklist. Chargers, adapters, toiletries, documents.",
            ),
            created_at: 920_000_000_000,
            updated_at: 1_020_000_000_000, // frequently reopened before trips
            deleted_at: None,
            version: 4,
        },
        // === PROJECTS ===
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000003").unwrap(),
            title: "Q3 Project Plan".to_string(),
            body: raki_domain::text_to_body(
                "Project roadmap for Q3. Milestones and owners. Pinned for daily reference.",
            ),
            created_at: 950_000_000_000,
            updated_at: 1_010_000_000_000,
            deleted_at: None,
            version: 5,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000007").unwrap(),
            title: "Q2 Project Plan".to_string(),
            body: raki_domain::text_to_body(
                "Archived Q2 roadmap. Completed milestones and retrospective notes.",
            ),
            created_at: 880_000_000_000,
            updated_at: 890_000_000_000, // old and untouched
            deleted_at: None,
            version: 2,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000008").unwrap(),
            title: "Garden Automation Project".to_string(),
            body: raki_domain::text_to_body(
                "Sensors, watering schedule, microcontroller notes for the garden project.",
            ),
            created_at: 960_000_000_000,
            updated_at: 995_000_000_000,
            deleted_at: None,
            version: 2,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000009").unwrap(),
            title: "Smart Home Dashboard".to_string(),
            body: raki_domain::text_to_body(
                "UI mockups and data sources for the smart home dashboard project.",
            ),
            created_at: 990_000_000_000,
            updated_at: 1_015_000_000_000,
            deleted_at: None,
            version: 3,
        },
        // === FINANCES (rounded, fake figures) ===
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000010").unwrap(),
            title: "Weekly Budget Review".to_string(),
            body: raki_domain::text_to_body(
                "Groceries 120, transport 45, utilities 90, dining 80. Total rounded to hundreds.",
            ),
            created_at: 1_005_000_000_000,
            updated_at: 1_045_000_000_000, // frequently reopened
            deleted_at: None,
            version: 8,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000011").unwrap(),
            title: "Monthly Investment Update".to_string(),
            body: raki_domain::text_to_body(
                "Portfolio allocation check. Index funds, bonds, and cash targets. Rounded percentages.",
            ),
            created_at: 975_000_000_000,
            updated_at: 1_025_000_000_000,
            deleted_at: None,
            version: 4,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000012").unwrap(),
            title: "Vacation Savings Goal".to_string(),
            body: raki_domain::text_to_body(
                "Target amount for upcoming travel. Monthly contribution plan, fake account nicknames.",
            ),
            created_at: 940_000_000_000,
            updated_at: 985_000_000_000,
            deleted_at: None,
            version: 2,
        },
        // === HEALTH (generic symptoms, no diagnoses or providers) ===
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000013").unwrap(),
            title: "Health Checkup 2025".to_string(),
            body: raki_domain::text_to_body(
                "Annual checkup notes 2025. Vitals, general fitness goals, follow-up reminders.",
            ),
            created_at: 1_020_000_000_000,
            updated_at: 1_035_000_000_000,
            deleted_at: None,
            version: 2,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000014").unwrap(),
            title: "Health Checkup 2023".to_string(),
            body: raki_domain::text_to_body(
                "Old annual checkup notes 2023. Baseline metrics and general observations.",
            ),
            created_at: 890_000_000_000,
            updated_at: 890_000_000_000, // old and untouched
            deleted_at: None,
            version: 1,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000015").unwrap(),
            title: "Doctor Visit 2024".to_string(),
            body: raki_domain::text_to_body(
                "Routine visit notes 2024. General symptoms discussed, no specific diagnosis recorded.",
            ),
            created_at: 950_000_000_000,
            updated_at: 955_000_000_000,
            deleted_at: None,
            version: 1,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000016").unwrap(),
            title: "Running Routine".to_string(),
            body: raki_domain::text_to_body(
                "Weekly running schedule. Easy runs, tempo, long run, rest days.",
            ),
            created_at: 980_000_000_000,
            updated_at: 1_040_000_000_000, // frequently reopened
            deleted_at: None,
            version: 6,
        },
        // === RECIPES ===
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000017").unwrap(),
            title: "Recipe: Sourdough Bread".to_string(),
            body: raki_domain::text_to_body(
                "Starter feeding schedule, autolyse, folds, bake temperature and timing.",
            ),
            created_at: 930_000_000_000,
            updated_at: 1_012_000_000_000,
            deleted_at: None,
            version: 4,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000018").unwrap(),
            title: "Recipe: Lentil Curry".to_string(),
            body: raki_domain::text_to_body(
                "Red lentils, coconut milk, curry powder, tomato, spinach. Quick weeknight recipe.",
            ),
            created_at: 910_000_000_000,
            updated_at: 920_000_000_000,
            deleted_at: None,
            version: 1,
        },
        // === BOOKS ===
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000019").unwrap(),
            title: "Book Notes: Design Systems".to_string(),
            body: raki_domain::text_to_body(
                "Key takeaways on tokens, components, and documentation from a design systems book.",
            ),
            created_at: 960_000_000_000,
            updated_at: 1_000_000_000_000,
            deleted_at: None,
            version: 3,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000020").unwrap(),
            title: "Book Notes: Distributed Systems".to_string(),
            body: raki_domain::text_to_body(
                "CAP theorem, consensus, replication, failure modes. Study notes.",
            ),
            created_at: 985_000_000_000,
            updated_at: 1_030_000_000_000,
            deleted_at: None,
            version: 2,
        },
        // === IDEAS ===
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000021").unwrap(),
            title: "Idea: Personal Habit Tracker".to_string(),
            body: raki_domain::text_to_body(
                "Minimal habit tracker idea. Streaks, categories, weekly review. Pinned for development.",
            ),
            created_at: 1_010_000_000_000,
            updated_at: 1_050_000_000_000, // frequently reopened
            deleted_at: None,
            version: 5,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000022").unwrap(),
            title: "Idea: Second Brain".to_string(),
            body: raki_domain::text_to_body(
                "Concept note for a second brain app. Capture, organize, retrieve, create.",
            ),
            created_at: 870_000_000_000,
            updated_at: 1_020_000_000_000,
            deleted_at: None,
            version: 4,
        },
        // === PEOPLE (fictional) ===
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000023").unwrap(),
            title: "Alex Rivera".to_string(),
            body: raki_domain::text_to_body(
                "Fictional contact. Likes hiking, sourdough, and board games. Birthday in October. Pinned.",
            ),
            created_at: 940_000_000_000,
            updated_at: 1_055_000_000_000, // frequently reopened
            deleted_at: None,
            version: 7,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000024").unwrap(),
            title: "Morgan Chen".to_string(),
            body: raki_domain::text_to_body(
                "Fictional colleague. Working on the smart home dashboard project. Feedback notes.",
            ),
            created_at: 995_000_000_000,
            updated_at: 1_015_000_000_000,
            deleted_at: None,
            version: 3,
        },
        // === MISC ===
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000025").unwrap(),
            title: "Grocery List Template".to_string(),
            body: raki_domain::text_to_body(
                "Produce, pantry, dairy, proteins, household. Reusable weekly template.",
            ),
            created_at: 900_000_000_000,
            updated_at: 1_008_000_000_000,
            deleted_at: None,
            version: 5,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000026").unwrap(),
            title: "Reflection: Annual Review 2024".to_string(),
            body: raki_domain::text_to_body(
                "Personal annual review. Wins, lessons, goals for 2025. No specific numbers.",
            ),
            created_at: 1_002_000_000_000,
            updated_at: 1_002_000_000_000,
            deleted_at: None,
            version: 1,
        },
    ]
}

/// IDs of notes that should be treated as pinned in evaluation scenarios.
/// The `Note` type does not carry a `pinned` field; pinning is an external signal.
pub fn seed_pinned() -> &'static [&'static str] {
    &[
        "00000000-0000-0000-0000-000000000003", // Q3 Project Plan
        "00000000-0000-0000-0000-000000000010", // Weekly Budget Review
        "00000000-0000-0000-0000-000000000013", // Health Checkup 2025
        "00000000-0000-0000-0000-000000000021", // Idea: Personal Habit Tracker
        "00000000-0000-0000-0000-000000000023", // Alex Rivera
    ]
}

/// Deterministic query cases covering recency, pin, and salience rescues.
pub fn seed_queries() -> Vec<QueryCase> {
    vec![
        QueryCase {
            query: "japan trip budget",
            expected_note_ids: &["00000000-0000-0000-0000-000000000001"],
            rationale: "Recency should lift Japan Trip 2025 above Japan Trip 2023.",
        },
        QueryCase {
            query: "japan trip",
            expected_note_ids: &["00000000-0000-0000-0000-000000000001"],
            rationale: "Recency rescue: the 2025 note should outrank the 2023 note.",
        },
        QueryCase {
            query: "project plan",
            expected_note_ids: &["00000000-0000-0000-0000-000000000003"],
            rationale:
                "Pinned Q3 plan should outrank the older Q2 plan and incidental project mentions.",
        },
        QueryCase {
            query: "quarterly roadmap",
            expected_note_ids: &["00000000-0000-0000-0000-000000000003"],
            rationale: "Pinned and more recent Q3 plan should beat Q2 archive.",
        },
        QueryCase {
            query: "health checkup",
            expected_note_ids: &["00000000-0000-0000-0000-000000000013"],
            rationale: "Recency + salience rescue: 2025 checkup should beat 2023.",
        },
        QueryCase {
            query: "annual checkup notes",
            expected_note_ids: &["00000000-0000-0000-0000-000000000013"],
            rationale: "The 2025 health checkup is the most recent and relevant.",
        },
        QueryCase {
            query: "doctor visit",
            expected_note_ids: &["00000000-0000-0000-0000-000000000015"],
            rationale: "Exact title match for the 2024 doctor visit note.",
        },
        QueryCase {
            query: "running schedule",
            expected_note_ids: &["00000000-0000-0000-0000-000000000016"],
            rationale: "Running Routine is frequently reopened and topically focused.",
        },
        QueryCase {
            query: "weekly budget",
            expected_note_ids: &["00000000-0000-0000-0000-000000000010"],
            rationale: "Pinned budget note should outrank investment and savings notes.",
        },
        QueryCase {
            query: "grocery spending",
            expected_note_ids: &["00000000-0000-0000-0000-000000000010"],
            rationale: "Budget review mentions groceries; pinned signal reinforces it.",
        },
        QueryCase {
            query: "investment allocation",
            expected_note_ids: &["00000000-0000-0000-0000-000000000011"],
            rationale: "Monthly Investment Update is the only note about portfolio allocation.",
        },
        QueryCase {
            query: "trip packing",
            expected_note_ids: &["00000000-0000-0000-0000-000000000006"],
            rationale: "Packing list is frequently reused across trips.",
        },
        QueryCase {
            query: "italy itinerary",
            expected_note_ids: &["00000000-0000-0000-0000-000000000004"],
            rationale: "Exact domain match for the Italy Trip 2024 note.",
        },
        QueryCase {
            query: "spain 2026",
            expected_note_ids: &["00000000-0000-0000-0000-000000000005"],
            rationale: "Year disambiguation should return the Spain Trip 2026 note.",
        },
        QueryCase {
            query: "sourdough recipe",
            expected_note_ids: &["00000000-0000-0000-0000-000000000017"],
            rationale: "Title match for the sourdough bread recipe.",
        },
        QueryCase {
            query: "lentil curry",
            expected_note_ids: &["00000000-0000-0000-0000-000000000018"],
            rationale: "Title match for the lentil curry recipe.",
        },
        QueryCase {
            query: "design systems notes",
            expected_note_ids: &["00000000-0000-0000-0000-000000000019"],
            rationale: "Book notes on design systems should rank above unrelated notes.",
        },
        QueryCase {
            query: "distributed systems study",
            expected_note_ids: &["00000000-0000-0000-0000-000000000020"],
            rationale: "CAP theorem and consensus content is unique to this note.",
        },
        QueryCase {
            query: "habit tracker idea",
            expected_note_ids: &["00000000-0000-0000-0000-000000000021"],
            rationale: "Pinned and frequently reopened habit tracker idea.",
        },
        QueryCase {
            query: "second brain concept",
            expected_note_ids: &["00000000-0000-0000-0000-000000000022"],
            rationale: "Idea note about the second brain concept.",
        },
        QueryCase {
            query: "Alex Rivera",
            expected_note_ids: &["00000000-0000-0000-0000-000000000023"],
            rationale: "Pinned fictional person note should rank at the top.",
        },
        QueryCase {
            query: "Morgan Chen feedback",
            expected_note_ids: &["00000000-0000-0000-0000-000000000024"],
            rationale: "Person note tied to project feedback.",
        },
        QueryCase {
            query: "grocery list",
            expected_note_ids: &["00000000-0000-0000-0000-000000000025"],
            rationale: "Template note for weekly groceries.",
        },
        QueryCase {
            query: "annual review 2024",
            expected_note_ids: &["00000000-0000-0000-0000-000000000026"],
            rationale: "Reflection note for the 2024 annual review.",
        },
        QueryCase {
            query: "smart home dashboard",
            expected_note_ids: &["00000000-0000-0000-0000-000000000009"],
            rationale: "Project-specific note should outrank the Morgan Chen mention.",
        },
        QueryCase {
            query: "garden automation",
            expected_note_ids: &["00000000-0000-0000-0000-000000000008"],
            rationale: "Exact project match for garden automation.",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_corpus_has_expected_count() {
        assert!(seed_notes().len() >= 25, "expected at least 25 seed notes");
        assert!(
            seed_queries().len() >= 20,
            "expected at least 20 seed queries"
        );
    }

    #[test]
    fn seed_query_ids_are_valid() {
        let ids: std::collections::HashSet<String> =
            seed_notes().into_iter().map(|n| n.id.to_string()).collect();

        for case in seed_queries() {
            for expected in case.expected_note_ids {
                assert!(
                    ids.contains(*expected),
                    "query '{}' expected unknown note id {}",
                    case.query,
                    expected
                );
            }
        }
    }
}
