CREATE TABLE board_rentals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    board_id TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    starts_at TIMESTAMPTZ NOT NULL,
    ends_at TIMESTAMPTZ NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    cancelled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (starts_at < ends_at)
);
CREATE INDEX board_rentals_schedule ON board_rentals(board_id, starts_at, ends_at) WHERE cancelled_at IS NULL;
CREATE INDEX board_rentals_user ON board_rentals(user_id);
ALTER TABLE session_owners ADD COLUMN rental_id UUID REFERENCES board_rentals(id);
