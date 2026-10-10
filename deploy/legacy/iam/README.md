# Historical IAM operator

`public-id-cutover.py` is retained for the retired IAM public-ID migration. It
requires a matching historical backend binary, IAM identity map, database schema,
and runtime configuration. Its `apply` action changes databases and is not an
Accounts migration or a current deployment step.

Current instructions are in [Deployment](../../../docs/DEPLOYMENT.md) and
[Apps](../../../docs/APPS.md). The archived offline invariant checks are in
`scripts/legacy/iam/test_public_id_cutover.py`.
