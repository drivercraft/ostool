import { useEffect } from "react";
import { Link, useParams } from "react-router-dom";
import { ArrowLeft, ArrowRight, Check, CircuitBoard } from "lucide-react";
import { site, useDeviceDocument, usePublicDevices } from "@/api/events";
import { useEdition } from "@/edition";
import { Button } from "@/components/ui/button";
import { Notice } from "@/components/forms";
import { Skeleton } from "@/components/ui/skeleton";
import { BoardVisual } from "@/apps/web/components/board-visual";

export default function DeviceDetail() {
  const { boardId = "" } = useParams();
  const devices = usePublicDevices();
  const selected = useDeviceDocument();
  const publicEdition = useEdition().enabled;
  useEffect(() => {
    site.setDeviceDocument(boardId);
    return () => site.setDeviceDocument(null);
  }, [boardId]);
  const board = devices?.find((item) => item.id === boardId);
  if (!devices)
    return (
      <main className="public-inner-page">
        <Skeleton className="h-96 w-full" />
      </main>
    );
  if (!board)
    return (
      <main className="public-inner-page">
        <Notice>
          该开发板不存在或已被移除。<Link to="/devices">返回设备列表</Link>
        </Notice>
      </main>
    );
  const document = selected?.id === boardId ? selected.document : null;
  const title = document?.title || board.id;
  return (
    <main className="public-inner-page device-detail-page">
      <nav className="device-detail-nav" aria-label="设备详情导航">
        <Link to="/devices">
          <ArrowLeft />
          全部设备
        </Link>
        <strong>{title}</strong>
        <div>
          <a href="#device-introduction">产品介绍</a>
          <a href="#device-specifications">规格参数</a>
          {board.disabled ? (
            <Button disabled>暂不可申请</Button>
          ) : (
            <Button asChild>
              <Link
                to={
                  publicEdition
                    ? `/rental-request?board=${encodeURIComponent(board.id)}`
                    : "/admin/boards"
                }
              >
                {publicEdition ? "申请租赁" : "打开控制台"}
              </Link>
            </Button>
          )}
        </div>
      </nav>
      <section
        className="device-detail-hero"
        aria-labelledby="device-detail-title"
      >
        <span className="section-label">
          {board.board_type} / DEVELOPMENT BOARD
        </span>
        <h1 id="device-detail-title">{title}</h1>
        <p>{document?.summary || "把下一次构建，带到真实的硬件上。"}</p>
        <div className="device-detail-tags">
          <span>{board.disabled ? "暂不可申请" : "已启用"}</span>
          {board.tags.map((tag) => (
            <span key={tag}>{tag}</span>
          ))}
        </div>
        <div className="device-detail-visual">
          {document?.image_url ? (
            <img
              src={document.image_url}
              alt={`${title} 开发板`}
              referrerPolicy="no-referrer"
            />
          ) : (
            <BoardVisual label={board.board_type} />
          )}
        </div>
        {!document?.image_url && (
          <small className="device-image-caption">
            开发板示意图 · 非设备实拍
          </small>
        )}
      </section>
      <section id="device-introduction" className="device-detail-introduction">
        <div>
          <span className="section-label">BUILT FOR YOUR NEXT IDEA</span>
          <h2>认识 {title}。</h2>
        </div>
        <div>
          {document?.description ? (
            <p className="device-document-prose">{document.description}</p>
          ) : (
            <p>
              设备介绍尚未发布。你可以查看已登记的板型、标签与设备状态，或联系管理员了解更多信息。
            </p>
          )}
          {!!document?.highlights.length && (
            <ul>
              {document.highlights.map((value, index) => (
                <li key={index}>
                  <Check />
                  {value}
                </li>
              ))}
            </ul>
          )}
        </div>
      </section>
      <section
        id="device-specifications"
        className="device-detail-specifications"
      >
        <div>
          <span className="section-label">THE DETAILS MATTER</span>
          <h2>规格与信息。</h2>
          <p>来自管理员发布的设备文档。</p>
        </div>
        <dl>
          {[
            { name: "设备标识", value: board.id },
            { name: "开发板类型", value: board.board_type },
            { name: "设备状态", value: board.disabled ? "已禁用" : "已启用" },
            ...(document?.specifications ?? []),
          ].map((item, index) => (
            <div key={index}>
              <dt>{item.name}</dt>
              <dd>{item.value}</dd>
            </div>
          ))}
        </dl>
      </section>
      <section className="device-detail-cta">
        <CircuitBoard />
        <h2>让想法，在硬件上运行。</h2>
        <p>
          {publicEdition
            ? "选择合适的使用时段，开始下一次开发与验证。"
            : "在本地控制台中，直接连接可用的开发板。"}
        </p>
        {board.disabled ? (
          <Button disabled>暂不可申请</Button>
        ) : (
          <Button asChild>
            <Link
              to={
                publicEdition
                  ? `/rental-request?board=${encodeURIComponent(board.id)}`
                  : "/admin/boards"
              }
            >
              {publicEdition ? "申请租赁" : "打开控制台"}
              <ArrowRight />
            </Link>
          </Button>
        )}
      </section>
    </main>
  );
}
