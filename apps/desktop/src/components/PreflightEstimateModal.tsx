import type { PreflightEstimate } from "../types";

interface PreflightEstimateModalProps {
  estimate: PreflightEstimate;
  title?: string;
  onConfirm?: () => void;
  onClose: () => void;
}

const number = new Intl.NumberFormat("zh-CN");

function duration(seconds: number) {
  if (seconds < 60) return `${seconds} 秒`;
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 60) return `${minutes} 分钟`;
  const hours = Math.floor(minutes / 60);
  const remainder = minutes % 60;
  return remainder === 0 ? `${hours} 小时` : `${hours} 小时 ${remainder} 分钟`;
}

export function PreflightEstimateModal({
  estimate,
  title = "执行前估算",
  onConfirm,
  onClose,
}: PreflightEstimateModalProps) {
  const cost =
    estimate.estimatedCostLow == null || estimate.estimatedCostHigh == null
      ? "Provider 未提供价格"
      : `$${estimate.estimatedCostLow.toFixed(4)} – $${estimate.estimatedCostHigh.toFixed(4)}`;

  return (
    <div className="nested-modal-backdrop" role="presentation">
      <section
        className="preflight-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="preflight-title"
      >
        <header>
          <div>
            <p className="eyebrow">PREFLIGHT ESTIMATE</p>
            <h3 id="preflight-title">{title}</h3>
          </div>
          <button
            className="close-button"
            type="button"
            aria-label="关闭估算"
            onClick={onClose}
          >
            ×
          </button>
        </header>
        <div className="estimate-grid">
          <span>处理深度<strong>第 {estimate.complexityLevel} 档</strong></span>
          <span>Skills<strong>{number.format(estimate.skillCount)}</strong></span>
          <span>LLM 文件<strong>{number.format(estimate.analysisFileCount)}</strong></span>
          <span>Embedding 文件<strong>{number.format(estimate.embeddingFileCount)}</strong></span>
          <span>Chunks<strong>{number.format(estimate.chunkCountLow)}–{number.format(estimate.chunkCountHigh)}</strong></span>
          <span>LLM Tokens<strong>{number.format(estimate.analysisTokensLow)}–{number.format(estimate.analysisTokensHigh)}</strong></span>
          <span>Embedding Tokens<strong>{number.format(estimate.embeddingTokensLow)}–{number.format(estimate.embeddingTokensHigh)}</strong></span>
          <span>预计时间<strong>{duration(estimate.estimatedSecondsLow)}–{duration(estimate.estimatedSecondsHigh)}</strong></span>
          <span>预估费用<strong>{cost}</strong></span>
          <span>置信度<strong>{estimate.confidence.toUpperCase()}</strong></span>
        </div>
        <p className="estimate-model">
          {estimate.provider} / {estimate.model}
        </p>
        {estimate.missingReasons.length > 0 && (
          <div className="inline-notice">
            {estimate.missingReasons.join("；")}
          </div>
        )}
        <footer>
          <button type="button" onClick={onClose}>关闭</button>
          {onConfirm && (
            <button className="primary-button" type="button" onClick={onConfirm}>
              确认执行
            </button>
          )}
        </footer>
      </section>
    </div>
  );
}
