-- 管理列表及个人中心按创建时间分页，主键作为相同时间记录的稳定排序条件。
CREATE INDEX board_rentals_created ON board_rentals(created_at DESC,id DESC);
CREATE INDEX board_rentals_user_created ON board_rentals(user_id,created_at DESC,id DESC);
CREATE INDEX rental_requests_created ON rental_requests(created_at DESC,id DESC);
CREATE INDEX rental_requests_status_created ON rental_requests(status,created_at DESC,id DESC);
DROP INDEX rental_requests_user;
CREATE INDEX rental_requests_user ON rental_requests(user_id,created_at DESC,id DESC);
