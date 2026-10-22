-- 硬件租约历史独立于活动连接；身份和设备名称保留分配时的快照。
-- 不使用级联外键，删除用户、设备或租赁不能抹去使用记录。
CREATE TABLE session_history (
    session_id TEXT PRIMARY KEY,
    board_id TEXT NOT NULL,
    board_type TEXT NOT NULL,
    user_id UUID,
    username TEXT,
    rental_id UUID,
    client_name TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    last_heartbeat_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    stop_requested_at TIMESTAMPTZ,
    ended_at TIMESTAMPTZ,
    stop_reason TEXT,
    end_reason TEXT,
    release_failures INTEGER NOT NULL DEFAULT 0 CHECK (release_failures >= 0),
    last_release_error TEXT,
    CHECK (ended_at IS NULL OR ended_at >= created_at),
    CHECK ((ended_at IS NULL) = (end_reason IS NULL))
);
CREATE INDEX session_history_created ON session_history(created_at DESC,session_id DESC);
CREATE INDEX session_history_board ON session_history(board_id,created_at DESC);
CREATE INDEX session_history_user ON session_history(user_id,created_at DESC);
CREATE INDEX session_history_open ON session_history(session_id) WHERE ended_at IS NULL;
