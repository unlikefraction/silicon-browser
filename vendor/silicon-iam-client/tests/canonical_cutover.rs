//! Read compatibility lets consumers deploy before the canonical IAM cutover.

#![allow(clippy::expect_used)]

use serde_json::json;
use silicon_iam_client::models;

#[test]
fn directory_job_description_reads_both_revisions_and_writes_only_current_name() {
    for name in ["job_role", "job_description"] {
        let role: models::DirectoryRole = serde_json::from_value(json!({
            "org_role":"member", name:"Maintains the directory"
        }))
        .expect("directory role from either deployed revision");
        assert_eq!(role.job_description, "Maintains the directory");
        assert_eq!(
            serde_json::to_value(role).expect("serialize"),
            json!({
                "org_role":"member", "job_description":"Maintains the directory"
            })
        );
    }
}

#[test]
fn honeycomb_recipients_accept_old_pages_and_canonical_cursors() {
    for name in ["principal_id", "carbon_id"] {
        let page: models::HoneycombOrganizationRecipients = serde_json::from_value(json!({
            "org_id":"tos", "recipients":[{name:"saket", "email":"saket@example.com"}],
            "next_cursor":"saket"
        }))
        .expect("recipient read during cutover");
        assert_eq!(page.next_cursor.as_deref(), Some("saket"));
        let output = serde_json::to_value(page).expect("serialize");
        assert_eq!(output["recipients"][0]["carbon_id"], "saket");
        assert!(output["recipients"][0].get("principal_id").is_none());
    }
}
