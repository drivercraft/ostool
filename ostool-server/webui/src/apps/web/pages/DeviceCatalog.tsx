import { useState, useSyncExternalStore } from "react";
import { CircuitBoard, Search } from "lucide-react";
import { Link } from "react-router-dom";
import { site, usePublicDevices } from "@/api/events";
import { useEdition } from "@/edition";
import { useSiteSession } from "@/apps/web/session";
import { Button } from "@/components/ui/button";
import { BoardVisual } from "@/apps/web/components/board-visual";
import { Skeleton } from "@/components/ui/skeleton";
import { Notice, TextField } from "@/components/forms";
import { useListPage } from "@/components/list-pagination";

/** Public projection is shared by visitors and members, with cards in both editions. */
export function DeviceCatalog() {
  const devices = usePublicDevices();
  const errors = useSyncExternalStore(site.subscribe, site.getErrors);
  const connection = useSyncExternalStore(site.subscribe, site.getConnection);
  const { user } = useSiteSession();
  const publicEdition = useEdition().edition === "public";
  const [search, setSearch] = useState("");
  const [type, setType] = useState("");
  const types = Array.from(
    new Set((devices ?? []).map((board) => board.board_type)),
  ).sort();
  const filtered = (devices ?? []).filter(
    (board) =>
      (!type || board.board_type === type) &&
      `${board.id} ${board.board_type} ${board.tags.join(" ")}`
        .toLowerCase()
        .includes(search.toLowerCase()),
  );
  const page = useListPage(filtered, JSON.stringify([search, type]));
  return (
    <section
      id="device-catalog"
      className="device-directory"
      aria-label="设备目录"
    >
      <div className="device-directory-tools">
        <div>
          <span className="section-label">EXPLORE THE HARDWARE</span>
          <h2>找到适合你的设备。</h2>
          <p>
            {devices
              ? `${devices.length} 台设备 · ${types.length} 种板型`
              : "正在同步设备目录…"}
          </p>
        </div>
        <TextField
          label="搜索设备"
          placeholder="名称、板型或标签"
          value={search}
          onValue={setSearch}
        />
      </div>
      {!!types.length && (
        <div
          className="device-category-tabs"
          role="group"
          aria-label="板型筛选"
        >
          <button
            type="button"
            aria-pressed={!type}
            onClick={() => setType("")}
          >
            全部设备
          </button>
          {types.map((value) => (
            <button
              key={value}
              type="button"
              aria-pressed={type === value}
              onClick={() => setType(value)}
            >
              {value}
            </button>
          ))}
        </div>
      )}
      {errors.device_catalog && <Notice>{errors.device_catalog}</Notice>}
      {connection === "reconnecting" && (
        <Notice>设备目录连接中断，正在重连…</Notice>
      )}
      {!devices ? (
        <div className="device-directory-grid" aria-label="正在加载设备">
          {[0, 1, 2].map((key) => (
            <Skeleton key={key} className="h-96 w-full" />
          ))}
        </div>
      ) : filtered.length ? (
        <div className="device-directory-grid">
          {page.rows.map((board) => {
            const application = `/account/requests/new?board=${encodeURIComponent(board.id)}`;
            return (
              <article className="device-directory-card" key={board.id}>
                <BoardVisual label={board.board_type} compact />
                <div className="device-directory-card-top">
                  <CircuitBoard aria-hidden="true" />
                  <span
                    className={
                      board.disabled ? "device-unavailable" : "device-enabled"
                    }
                  >
                    {board.disabled ? "暂不可用" : "已启用"}
                  </span>
                </div>
                <h3>{board.id}</h3>
                <p>{board.board_type}</p>
                <div className="device-directory-tags">
                  {board.tags.map((tag) => (
                    <span key={tag}>{tag}</span>
                  ))}
                </div>
                {publicEdition ? (
                  board.disabled ? (
                    <Button disabled>暂不可申请</Button>
                  ) : (
                    <Button asChild>
                      <Link
                        to={
                          user
                            ? application
                            : `/login?redirect=${encodeURIComponent(application)}`
                        }
                      >
                        {user ? "申请租赁" : "登录后申请"}
                      </Link>
                    </Button>
                  )
                ) : (
                  <Button asChild variant="outline">
                    <a href="/admin/boards">查看设备</a>
                  </Button>
                )}
              </article>
            );
          })}
        </div>
      ) : (
        <div className="device-directory-empty" role="status">
          {search ? (
            <Search aria-hidden="true" />
          ) : (
            <CircuitBoard aria-hidden="true" />
          )}
          <p>{search ? "没有匹配的设备" : "暂无设备"}</p>
        </div>
      )}
      {devices && <div className="device-directory-footer">{page.footer}</div>}
    </section>
  );
}
