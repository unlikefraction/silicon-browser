# Historical IAM checks

These preserved scripts target the retired IAM authentication, testing, and
public-identifier contracts. They are excluded from current CI and do not verify
the Accounts backend. The live harnesses require the matching historical backend
and credentials; do not run them against the current service.

`test_public_id_cutover.py` checks the archived operator in
`deploy/legacy/iam/public-id-cutover.py`. Current release and authentication
instructions are in [Apps](../../../docs/APPS.md) and
[Deployment](../../../docs/DEPLOYMENT.md).
