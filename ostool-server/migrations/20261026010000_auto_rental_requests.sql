ALTER TABLE rental_requests DROP CONSTRAINT rental_requests_status_check;
ALTER TABLE rental_requests ADD CONSTRAINT rental_requests_status_check
    CHECK (status IN ('pending','approved','auto_approved','rejected','cancelled'));

INSERT INTO rental_requests(user_id,board_id,starts_at,ends_at,notes,status,decision_note,reviewed_at,rental_id,created_at)
SELECT r.user_id,r.board_id,r.starts_at,r.ends_at,r.notes,
       CASE WHEN r.cancelled_at IS NULL THEN 'auto_approved' ELSE 'cancelled' END,
       '自动放行',r.created_at,r.id,r.created_at
FROM board_rentals r
WHERE r.source='instant'
  AND NOT EXISTS (SELECT 1 FROM rental_requests q WHERE q.rental_id=r.id);
