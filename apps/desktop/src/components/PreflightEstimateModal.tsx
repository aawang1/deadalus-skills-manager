import { tr, getLanguage } from "../i18n";
import type { PreflightEstimate } from "../types";
interface PreflightEstimateModalProps {
    estimate: PreflightEstimate;
    title?: string;
    onConfirm?: () => void;
    onClose: () => void;
}
const number = { format: (value: number) => new Intl.NumberFormat(getLanguage() === "zh" ? "zh-CN" : "en-US").format(value) };
function duration(seconds: number) {
    if (seconds < 60)
        return tr("{0} 秒", seconds);
    const minutes = Math.ceil(seconds / 60);
    if (minutes < 60)
        return tr("{0} 分钟", minutes);
    const hours = Math.floor(minutes / 60);
    const remainder = minutes % 60;
    return remainder === 0 ? tr("{0} 小时", hours) : tr("{0} 小时 {1} 分钟", hours, remainder);
}
export function PreflightEstimateModal({ estimate, title = tr("执行前估算"), onConfirm, onClose, }: PreflightEstimateModalProps) {
    const cost = estimate.estimatedCostLow == null || estimate.estimatedCostHigh == null
        ? tr("Provider 未提供价格") : `$${estimate.estimatedCostLow.toFixed(4)} – $${estimate.estimatedCostHigh.toFixed(4)}`;
    return (<div className="nested-modal-backdrop" role="presentation">
      <section className="preflight-modal" role="dialog" aria-modal="true" aria-labelledby="preflight-title">
        <header>
          <div>
            <p className="eyebrow">{tr("预检估算")}</p>
            <h3 id="preflight-title">{title}</h3>
          </div>
          <button className="close-button" type="button" aria-label={tr("关闭估算")} onClick={onClose}>
            ×
          </button>
        </header>
        <div className="estimate-grid">
          <span>{tr("处理深度")}<strong>{tr("第")}{estimate.complexityLevel}{tr("档")}</strong></span>
          <span>Skills<strong>{number.format(estimate.skillCount)}</strong></span>
          <span>{tr("LLM 文件")}<strong>{number.format(estimate.analysisFileCount)}</strong></span>
          <span>{tr("Embedding 文件")}<strong>{number.format(estimate.embeddingFileCount)}</strong></span>
          <span>{tr("分块")}<strong>{number.format(estimate.chunkCountLow)}–{number.format(estimate.chunkCountHigh)}</strong></span>
          <span>{tr("LLM Token 数")}<strong>{number.format(estimate.analysisTokensLow)}–{number.format(estimate.analysisTokensHigh)}</strong></span>
          <span>{tr("Embedding Token 数")}<strong>{number.format(estimate.embeddingTokensLow)}–{number.format(estimate.embeddingTokensHigh)}</strong></span>
          <span>{tr("预计时间")}<strong>{duration(estimate.estimatedSecondsLow)}–{duration(estimate.estimatedSecondsHigh)}</strong></span>
          <span>{tr("预估费用")}<strong>{cost}</strong></span>
          <span>{tr("置信度")}<strong>{estimate.confidence.toUpperCase()}</strong></span>
        </div>
        <p className="estimate-model">
          {estimate.provider} / {estimate.model}
        </p>
        {estimate.missingReasons.length > 0 && (<div className="inline-notice">
            {estimate.missingReasons.join("；")}
          </div>)}
        <footer>
          <button type="button" onClick={onClose}>{tr("关闭")}</button>
          {onConfirm && (<button className="primary-button" type="button" onClick={onConfirm}>{tr("确认执行")}</button>)}
        </footer>
      </section>
    </div>);
}
