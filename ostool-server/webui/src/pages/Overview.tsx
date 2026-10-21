import { PagePanel } from "@/components/page-panel";
import { ListToolbar } from "@/components/list-toolbar";
import { Button } from "@/components/ui/button";
import { Link } from "react-router-dom";
import { useResource } from "@/api/events";
import { Skeleton } from "@/components/ui/skeleton";
import { DataTable, Row, Cell, ReadonlyInfo } from "@/components/forms";
import { TftpState } from "./Settings";
export default function Overview() {
  const overview = useResource("overview");
  return (
    <PagePanel
      toolbar={
        <ListToolbar
          actions={
            <Button asChild>
              <Link to="/boards/new">新建开发板</Link>
            </Button>
          }
        />
      }
      footer={
        <span className="panel-status">
          {overview
            ? `${overview.board_count_total} 台开发板 · ${overview.board_count_available} 台可用 · ${overview.active_session_count} 个会话`
            : "正在同步…"}
        </span>
      }
    >
      {!overview ? (
        <Skeleton className="h-40 w-full" />
      ) : (
        <>
          <div className="metrics">
            {[
              ["开发板", overview.board_count_total, "/boards"],
              ["可用开发板", overview.board_count_available, "/boards"],
              ["已禁用", overview.disabled_board_count, "/boards"],
              ["活跃会话", overview.active_session_count, "/sessions"],
            ].map(([label, value, path]) => (
              <Link key={label} to={String(path)}>
                <span>{label}</span>
                <strong>{value}</strong>
              </Link>
            ))}
          </div>
          <section className="page-section">
            <h2>板型资源</h2>
            <DataTable
              headers={["板型", "标签", "总数", "可用"]}
              empty={!overview.board_types.length}
            >
              {overview.board_types.map((t) => (
                <Row key={t.board_type}>
                  <Cell>{t.board_type}</Cell>
                  <Cell>{t.tags.join(" · ") || "—"}</Cell>
                  <Cell>{t.total}</Cell>
                  <Cell>{t.available}</Cell>
                </Row>
              ))}
            </DataTable>
          </section>
          <section className="page-section">
            <h2>TFTP 状态</h2>
            <TftpState status={overview.tftp_status} />
          </section>
          <section className="page-section">
            <h2>服务器</h2>
            <ReadonlyInfo
              entries={{
                监听地址: overview.server.listen_addr,
                数据目录: overview.server.data_dir,
                板卡目录: overview.server.board_dir,
                "DTB 目录": overview.server.dtb_dir,
                "HTTP Boot 公共地址": overview.server.http_boot_public_base_url,
                "DTB 上传上限": `${overview.server.dtb_upload_max_mib} MiB`,
              }}
            />
          </section>
        </>
      )}
    </PagePanel>
  );
}
