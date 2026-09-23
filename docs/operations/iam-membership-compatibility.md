# IAM membership ID compatibility

Historical SDK 2/3 procedure. For IAM 4 and current Browser releases, use [the public identifier procedure](../PUBLIC-IDENTIFIER-MIGRATION.md) and `deploy/public-id-cutover.py`.

Browser's backend uses `silicon-iam-client` 2.0.0. IAM membership IDs are opaque
strings such as `chef:bricks[bricks]`; they must not be parsed as UUIDs. Principal,
organization, and session UUIDs retain their existing types.

The previous 1.9.0 client could reject a successful IAM introspection response
with `iam_contract` during login when membership IDs used the current format.
Browser now preserves membership IDs as strings in authentication and recording
delivery authority. Existing database membership columns already store text.

This is a backend compatibility fix; it does not require a CLI package update.
