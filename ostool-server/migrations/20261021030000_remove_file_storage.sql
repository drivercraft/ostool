-- Runtime storage has a single PostgreSQL backend. Keep business records intact.
ALTER TABLE dtb_records ALTER COLUMN content SET NOT NULL;
ALTER TABLE dtb_records DROP COLUMN storage_path;
DROP TABLE storage_imports;
