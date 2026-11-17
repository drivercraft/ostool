-- axloader's device LAN HTTP endpoint does not require TLS or a Bearer credential.
-- Existing configured credentials remain encrypted and retain their meaning.
ALTER TABLE device_connections ALTER COLUMN token_ciphertext DROP NOT NULL;
