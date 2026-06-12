//! Hand-authored synthetic real-notes corpus for R4 measurement.
//! All content is fictional and anonymized.

use raki_domain::{Note, NoteId};

pub struct QueryCase {
    pub query: &'static str,
    pub expected_note_ids: &'static [&'static str],
    pub rationale: &'static str,
}

pub fn seed_notes() -> Vec<Note> {
    vec![
        // Intentionally kept minimal in plan; expand to ~25 notes during implementation.
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000001").unwrap(),
            title: "Japan Trip 2025".to_string(),
            body: raki_domain::text_to_body(
                "Planning budget for Japan trip 2025. Tokyo, Kyoto, Osaka. Ryokan cash tips.",
            ),
            created_at: 1_000_000_000_000,
            updated_at: 1_000_000_000_000,
            deleted_at: None,
            version: 1,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000002").unwrap(),
            title: "Japan Trip 2023".to_string(),
            body: raki_domain::text_to_body("Old Japan trip notes from 2023. Osaka food tour."),
            created_at: 900_000_000_000,
            updated_at: 900_000_000_000,
            deleted_at: None,
            version: 1,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000003").unwrap(),
            title: "Q3 Project Plan".to_string(),
            body: raki_domain::text_to_body("Project roadmap for Q3. Milestones and owners."),
            created_at: 950_000_000_000,
            updated_at: 950_000_000_000,
            deleted_at: None,
            version: 1,
        },
    ]
}

pub fn seed_queries() -> Vec<QueryCase> {
    vec![
        QueryCase {
            query: "japan trip budget",
            expected_note_ids: &["00000000-0000-0000-0000-000000000001"],
            rationale: "Recency should lift Japan Trip 2025 above 2023.",
        },
        QueryCase {
            query: "project plan",
            expected_note_ids: &["00000000-0000-0000-0000-000000000003"],
            rationale: "Pinned Q3 plan should outrank incidental 'project' mentions.",
        },
    ]
}
