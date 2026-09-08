import { type FormEvent, useEffect, useMemo, useState } from "react";
import { api } from "../api";
import type {
  AgentId,
  InstalledSkill,
  SemanticSearchResult,
  VectorType,
} from "../types";

interface SemanticSearchProps {
  isNative: boolean;
  skills: InstalledSkill[];
}

const vectorTypeLabels: Record<VectorType, string> = {
  overall_function: "整体功能",
  trigger: "触发条件",
  workflow: "工作流",
  resource: "资源",
  general: "综合",
};

export function SemanticSearch({ isNative, skills }: SemanticSearchProps) {
  const [query, setQuery] = useState("");
  const [agent, setAgent] = useState<"all" | AgentId>("all");
  const [vectorType, setVectorType] = useState<"all" | VectorType>("all");
  const [results, setResults] = useState<SemanticSearchResult[]>([]);
  const [state, setState] = useState<
    "idle" | "loading" | "ready" | "error"
  >("idle");
  const [error, setError] = useState("");
  const [includeDisabled, setIncludeDisabled] = useState(false);
  const [preferenceBusy, setPreferenceBusy] = useState(false);
  useEffect(() => {
    if (!isNative) return;
    api.getSearchPreferences()
      .then((preferences) => setIncludeDisabled(preferences.includeDisabledSkills))
      .catch(() => undefined);
  }, [isNative]);
  const skillNames = useMemo(
    () => new Map(skills.map((skill) => [skill.skillId, skill.name])),
    [skills],
  );
  const hasHistory =
    query.length > 0 ||
    agent !== "all" ||
    vectorType !== "all" ||
    state !== "idle" ||
    results.length > 0 ||
    error.length > 0;

  const search = async (event: FormEvent) => {
    event.preventDefault();
    if (!isNative || !query.trim()) return;
    setState("loading");
    setError("");
    try {
      setResults(
        await api.semanticSearch(
          query.trim(),
          agent === "all" ? undefined : agent,
          vectorType === "all" ? undefined : [vectorType],
          includeDisabled,
        ),
      );
      setState("ready");
    } catch (nextError) {
      setResults([]);
      setError(String(nextError));
      setState("error");
    }
  };

  const clearHistory = () => {
    if (
      !hasHistory ||
      !window.confirm(
        "确定清空语义搜索历史吗？当前查询、筛选和结果将被清除。",
      )
    ) {
      return;
    }
    setQuery("");
    setAgent("all");
    setVectorType("all");
    setResults([]);
    setState("idle");
    setError("");
  };

  return (
    <section className="semantic-workspace" aria-label="语义搜索">
      <div className="semantic-search-heading">
        <div>
          <p className="eyebrow">SEMANTIC RETRIEVAL</p>
          <h2>查找适合当前任务的 Skill</h2>
          <span>按功能、触发条件与工作流匹配本地索引。</span>
        </div>
        <button
          className="semantic-clear-history"
          type="button"
          disabled={!hasHistory || state === "loading"}
          onClick={clearHistory}
        >
          清空历史
        </button>
      </div>

      <form className="semantic-search-form" onSubmit={search}>
        <label className="semantic-query">
          <span className="sr-only">搜索描述</span>
          <input
            type="search"
            value={query}
            placeholder={
              isNative ? "描述你想完成的任务…" : "桌面端可使用语义搜索"
            }
            disabled={!isNative || state === "loading"}
            onChange={(event) => setQuery(event.currentTarget.value)}
          />
        </label>
        <label>
          <span className="sr-only">Agent 筛选</span>
          <select
            aria-label="Agent 筛选"
            value={agent}
            disabled={!isNative || state === "loading"}
            onChange={(event) =>
              setAgent(event.currentTarget.value as "all" | AgentId)
            }
          >
            <option value="all">所有 Agent</option>
            <option value="claude-code">Claude Code</option>
            <option value="cursor">Cursor</option>
            <option value="codex">Codex</option>
          </select>
        </label>
        <label>
          <span className="sr-only">语义类型筛选</span>
          <select
            aria-label="语义类型筛选"
            value={vectorType}
            disabled={!isNative || state === "loading"}
            onChange={(event) =>
              setVectorType(event.currentTarget.value as "all" | VectorType)
            }
          >
            <option value="all">所有类型</option>
            {Object.entries(vectorTypeLabels).map(([id, label]) => (
              <option key={id} value={id}>{label}</option>
            ))}
          </select>
        </label>
        <button
          className="primary-button"
          type="submit"
          disabled={!isNative || !query.trim() || state === "loading"}
        >
          {state === "loading" ? "检索中…" : "搜索"}
        </button>
      </form>
      <label className="semantic-disabled-toggle">
        <input
          type="checkbox"
          checked={includeDisabled}
          disabled={!isNative || state === "loading" || preferenceBusy}
          onChange={async (event) => {
            const enabled = event.currentTarget.checked;
            setIncludeDisabled(enabled);
            setPreferenceBusy(true);
            try {
              const preferences = await api.setIncludeDisabledSkills(enabled);
              setIncludeDisabled(preferences.includeDisabledSkills);
            } catch (nextError) {
              setIncludeDisabled(!enabled);
              setError(String(nextError));
              setState("error");
            } finally {
              setPreferenceBusy(false);
            }
          }}
        />
        <span>包含当前 Agent 中已禁用的 Skills</span>
      </label>

      {!isNative && (
        <div className="semantic-state">
          <p>浏览器预览为只读</p>
          <span>语义搜索需要本地 Profile 和系统凭据，原生调用已禁用。</span>
        </div>
      )}
      {isNative && state === "idle" && (
        <div className="semantic-state">
          <p>输入任务描述开始搜索</p>
          <span>结果将展示匹配原因和可追溯证据。</span>
        </div>
      )}
      {state === "loading" && (
        <div className="semantic-state">
          <span className="spinner" />
          <p>正在检索本地语义索引…</p>
        </div>
      )}
      {state === "error" && (
        <div className="semantic-state semantic-state--error" role="alert">
          <p>搜索失败</p>
          <span>{error}</span>
        </div>
      )}
      {state === "ready" && results.length === 0 && (
        <div className="semantic-state" data-testid="semantic-zero-result">
          <p>没有匹配结果</p>
          <span>尝试更换描述或放宽 Agent / 类型筛选。</span>
        </div>
      )}
      {state === "ready" && results.length > 0 && (
        <ol className="semantic-results">
          {results.map((result) => (
            <li key={result.skillId}>
              <header>
                <div>
                  <h3>{skillNames.get(result.skillId) ?? result.skillId}</h3>
                  <code>{result.skillId}</code>
                </div>
                <div className="semantic-score">
                  {result.expired && (
                    <span className="expired-badge">索引已过期</span>
                  )}
                  {agent !== "all" &&
                    skills.find((skill) => skill.skillId === result.skillId)?.disabledAgents.includes(agent) && (
                      <span className="disabled-badge">已禁用</span>
                    )}
                  <strong>{(result.score * 100).toFixed(1)}%</strong>
                </div>
              </header>
              <div className="match-reasons">
                {result.matchedTypes.map((type) => (
                  <span key={type}>{vectorTypeLabels[type]}</span>
                ))}
              </div>
              <details>
                <summary>匹配证据 · {result.evidence.length}</summary>
                <ul className="search-evidence">
                  {result.evidence.map((evidence) => (
                    <li key={evidence.embeddingId}>
                      <span>{vectorTypeLabels[evidence.vectorType]} / {evidence.level}</span>
                      <strong>{evidence.rawScore.toFixed(3)}</strong>
                      <small>
                        {evidence.headingPath || evidence.sourceFile || "Skill 主记录"}
                        {evidence.expired ? " · 已过期" : ""}
                      </small>
                    </li>
                  ))}
                </ul>
              </details>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
