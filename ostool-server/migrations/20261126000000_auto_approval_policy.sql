-- Upgrade the former default limits, preserving explicitly customized policies.
ALTER TABLE roles ALTER COLUMN allocation_policy SET DEFAULT
    '{"max_duration_minutes":480,"max_concurrent_sessions":2,"min_available_minutes":30,"board_types":[]}';
UPDATE roles SET allocation_policy = allocation_policy ||
    '{"max_duration_minutes":480,"max_concurrent_sessions":2,"min_available_minutes":30}'::jsonb
WHERE allocation_policy @> '{"max_duration_minutes":120,"max_concurrent_sessions":1,"min_available_minutes":10}'::jsonb;
