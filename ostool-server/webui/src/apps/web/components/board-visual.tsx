import { CircuitBoard, Cpu, Network, Terminal } from "lucide-react";

/** Native illustration of the board workflow; no synthetic inventory statistics. */
export function BoardVisual({
  label = "HARDWARE WORKSPACE",
  compact = false,
}: {
  label?: string;
  compact?: boolean;
}) {
  return (
    <div
      className={`board-visual${compact ? " compact" : ""}`}
      aria-hidden="true"
    >
      <div className="board-visual-grid" />
      <div className="board-visual-caption">
        <span>ostool / hardware</span>
        <i /> BUILD · BOOT · DEBUG
      </div>
      <div className="board-assembly">
        <div className="board-pcb">
          <i className="board-hole top-left" />
          <i className="board-hole top-right" />
          <i className="board-hole bottom-left" />
          <i className="board-hole bottom-right" />
          <div className="board-circuit-lines" />
          <div className="board-chip">
            <Cpu />
            <strong>OS</strong>
            <span>DEVELOPMENT</span>
          </div>
          <div className="board-connector">
            <Network />
          </div>
          <div className="board-memory">
            <i />
            <i />
            <i />
          </div>
          <div className="board-pins">
            {Array.from({ length: 16 }, (_, index) => (
              <i key={index} />
            ))}
          </div>
          <div className="board-silkscreen">
            <CircuitBoard />
            {label}
          </div>
        </div>
        {!compact && (
          <div className="board-terminal">
            <Terminal />
            <code>build → boot → connect</code>
            <span>统一设备工作流</span>
          </div>
        )}
      </div>
      <div className="board-visual-protocols">
        <span>UART</span>
        <span>U-Boot</span>
        <span>HTTP Boot</span>
        <span>QEMU</span>
      </div>
    </div>
  );
}
