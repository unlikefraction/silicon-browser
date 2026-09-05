//! Explicit, one-shot sandbox upload through Browser's IAM and Briefcase adapters.
//! Reads secrets from the caller's environment and never writes credentials.

use silicon_browser_backend::auth::{IdentityProvider, RecordingProofRequest, SiliconIamIdentityProvider};
use silicon_browser_backend::providers::BriefcaseClient;
use uuid::Uuid;

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).ok().filter(|v| !v.is_empty()).ok_or_else(|| format!("{name} is required"))
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Briefcase sandbox upload failed: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let proof_only = args == ["--proof-only"];
    if !args.is_empty() && !proof_only {
        return Err("usage: briefcase_upload [--proof-only]".into());
    }
    // Both test keys are mandatory for uploads. Proof-only never contacts Briefcase.
    let iam_key = required("IAM_TEST_ENVIRONMENT_KEY")?;
    let briefcase_key = if proof_only { None } else { Some(required("BRIEFCASE_TEST_ENVIRONMENT_KEY")?) };
    let iam_url = std::env::var("SILICON_IAM_URL").unwrap_or_else(|_| "https://backend.iam.teamofsilicons.com".into());
    let briefcase_url =
        std::env::var("BRIEFCASE_URL").unwrap_or_else(|_| "https://backend.briefcase.teamofsilicons.com".into());
    let app_id = required("IAM_APP_ID")?;
    let org_id = required("SB_TEST_ORG")?;
    let actor_id = required("SB_TEST_ACTOR")?;
    let token = required("SB_AUTHTOKEN")?;
    let audience = required("BRIEFCASE_APP_ID")?;
    let input = required("SB_TEST_UPLOAD_FILE")?;
    let name = required("SB_TEST_UPLOAD_NAME")?;
    let content_type = std::env::var("SB_TEST_UPLOAD_CONTENT_TYPE").unwrap_or_else(|_| "text/plain".into());
    let max_bytes = match std::env::var("SB_TEST_UPLOAD_MAX_BYTES") {
        Ok(value) => value.parse::<usize>().map_err(|_| "SB_TEST_UPLOAD_MAX_BYTES must be a positive integer")?,
        Err(std::env::VarError::NotPresent) => 64 * 1024 * 1024,
        Err(_) => return Err("SB_TEST_UPLOAD_MAX_BYTES is not valid text".into()),
    };
    let briefcase = BriefcaseClient::with_upload_limit(&briefcase_url, briefcase_key.as_deref(), max_bytes)
        .map_err(|e| e.to_string())?;
    // Stage an immutable regular file before invoking this example. Hash and upload
    // the same open handle so replacing its pathname cannot change the proof body.
    let mut file = tokio::fs::File::open(&input).await.map_err(|_| "could not open sandbox upload file")?;
    let (digest, size) = briefcase.hash_file(&mut file).await.map_err(|e| e.to_string())?;
    let iam = SiliconIamIdentityProvider::connect_with_environment(
        &iam_url,
        app_id.clone(),
        required("IAM_APP_SECRET")?,
        Some(&iam_key),
    )
    .await
    .map_err(|e| e.to_string())?;
    let proof = iam
        .issue_recording_proof(
            &token,
            RecordingProofRequest {
                expected_org_id: org_id.clone(),
                expected_actor_id: actor_id,
                audience,
                path: String::new(),
                name,
                content_type,
                body_sha256: digest.clone(),
                idempotency_key: Uuid::now_v7().to_string(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    if proof_only {
        println!(
            "{}",
            serde_json::json!({"proof_id": proof.proof_id, "expires_at": proof.expires_at, "sha256": digest, "uploaded": false})
        );
        return Ok(());
    }
    let entry = briefcase.upload_file(&org_id, &app_id, &proof.grant, file, size).await.map_err(|e| e.to_string())?;
    println!("{}", serde_json::json!({"entry": entry, "sha256": digest}));
    Ok(())
}
