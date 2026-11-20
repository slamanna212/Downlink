-- Pinned leaf-certificate fingerprint (SHA-256 hex) for sources that allow a self-signed certificate.
-- A fingerprint is not a secret. NULL means not pinned yet (pinned on the next connect).
ALTER TABLE source ADD COLUMN tls_pin TEXT;
