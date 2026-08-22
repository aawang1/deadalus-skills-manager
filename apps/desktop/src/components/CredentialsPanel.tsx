import {
  type FormEvent,
  type RefObject,
  useState,
} from "react";
import { api } from "../api";
import type {
  ApiKeyMetadata,
  ProfileChangeRequest,
  ProviderId,
  ProviderModel,
} from "../types";

const providerNames: Record<ProviderId, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  deepseek: "DeepSeek",
  qwen: "Qwen",
};

interface CredentialsPanelProps {
  isNative: boolean;
  keys: ApiKeyMetadata[];
  setKeys: (keys: ApiKeyMetadata[]) => void;
  keyInputRef: RefObject<HTMLInputElement | null>;
  activeProfileProvider?: ProviderId;
  notify: (type: "info" | "success" | "error", message: string) => void;
  requestProfileSwitch: (request: ProfileChangeRequest) => void;
}

export function CredentialsPanel({
  isNative,
  keys,
  setKeys,
  keyInputRef,
  activeProfileProvider,
  notify,
  requestProfileSwitch,
}: CredentialsPanelProps) {
  const [provider, setProvider] = useState<ProviderId>("anthropic");
  const [apiKey, setApiKey] = useState("");
  const [busyId, setBusyId] = useState<string>();
  const [saving, setSaving] = useState(false);
  const [models, setModels] = useState<Record<string, ProviderModel[]>>({});
  const [loadingModels, setLoadingModels] = useState<string>();

  const guardNative = () => {
    if (isNative) return true;
    notify("info", "浏览器预览为只读；请在 Tauri 桌面窗口中管理系统凭据。");
    return false;
  };

  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (!apiKey.trim() || !guardNative()) return;
    setSaving(true);
    try {
      const saved = await api.saveApiKey(provider, apiKey);
      setKeys([...keys, saved]);
      setApiKey("");
      notify("success", "验证成功，凭据已安全保存。");
      keyInputRef.current?.focus();
    } catch (error) {
      notify("error", String(error));
    } finally {
      setSaving(false);
    }
  };

  const activate = async (
    key: ApiKeyMetadata,
    purpose: "agent" | "embedding",
  ) => {
    if (!guardNative()) return;
    const commit = async () => {
      setBusyId(key.id);
      try {
        setKeys(await api.activateApiKeyForPurpose(key.id, purpose));
        notify(
          "success",
          purpose === "agent" ? "Agent 凭据已切换。" : "Embedding 凭据已切换。",
        );
      } finally {
        setBusyId(undefined);
      }
    };
    if (
      purpose === "embedding" &&
      activeProfileProvider &&
      activeProfileProvider !== key.provider
    ) {
      requestProfileSwitch(
        {
          provider: key.provider,
          targetCredentialId: key.id,
          reason: "credential",
        },
      );
      return;
    }
    try {
      await commit();
    } catch (error) {
      notify("error", String(error));
    }
  };

  const remove = async (key: ApiKeyMetadata) => {
    if (
      !guardNative() ||
      !window.confirm(
        `确定删除 ${providerNames[key.provider]} ${key.maskedKey} 吗？`,
      )
    ) return;
    setBusyId(key.id);
    try {
      setKeys(await api.deleteApiKey(key.id));
      notify("success", "凭据已从系统凭据库删除。");
    } catch (error) {
      notify("error", String(error));
    } finally {
      setBusyId(undefined);
    }
  };

  const loadModels = async (key: ApiKeyMetadata) => {
    if (!guardNative()) return;
    setLoadingModels(key.id);
    try {
      const response = await api.listProviderModels(key.id);
      setModels((current) => ({ ...current, [key.id]: response.models }));
      if (!key.agentModel && response.models[0]) {
        setKeys(await api.setAgentModel(key.id, response.models[0].id));
      }
    } catch (error) {
      notify("error", String(error));
    } finally {
      setLoadingModels(undefined);
    }
  };

  const selectModel = async (key: ApiKeyMetadata, model: string) => {
    if (!guardNative()) return;
    setBusyId(key.id);
    try {
      setKeys(await api.setAgentModel(key.id, model));
      notify("success", "Agent 模型已保存。");
    } catch (error) {
      notify("error", String(error));
    } finally {
      setBusyId(undefined);
    }
  };

  const embeddingKeys = keys.filter(
    (key) => key.provider === "openai" || key.provider === "qwen",
  );

  return (
    <div className="settings-panel credentials-panel">
      <form className="key-form" onSubmit={save}>
        <label>
          <span>Provider</span>
          <select
            value={provider}
            disabled={!isNative || saving}
            onChange={(event) =>
              setProvider(event.currentTarget.value as ProviderId)
            }
          >
            {Object.entries(providerNames).map(([id, name]) => (
              <option key={id} value={id}>{name}</option>
            ))}
          </select>
        </label>
        <label className="key-input-label">
          <span>API Key</span>
          <input
            ref={keyInputRef}
            type="password"
            value={apiKey}
            autoComplete="off"
            spellCheck={false}
            placeholder={isNative ? "输入 API Key" : "桌面端可用"}
            disabled={!isNative || saving}
            onChange={(event) => setApiKey(event.currentTarget.value)}
          />
        </label>
        <button
          className="send-button"
          type="submit"
          disabled={!isNative || !apiKey.trim() || saving}
        >
          {saving ? "验证中" : "安全保存"}
        </button>
      </form>

      {!isNative && (
        <div className="inline-notice">
          浏览器预览不会调用 Credential Manager；原生操作已禁用。
        </div>
      )}

      <div className="credential-columns">
        <CredentialColumn title="Agent 凭据" count={keys.length}>
          {keys.length === 0 ? (
            <EmptyCredentials />
          ) : keys.map((key) => (
            <article className="credential-card" key={key.id}>
              <CredentialHeading
                item={key}
                active={key.isAgentActive}
                activeLabel="Agent 使用中"
              />
              <div className="credential-model">
                {models[key.id] ? (
                  <select
                    aria-label={`${providerNames[key.provider]} Agent 模型`}
                    value={key.agentModel ?? models[key.id][0]?.id ?? ""}
                    disabled={!isNative || busyId === key.id}
                    onChange={(event) =>
                      selectModel(key, event.currentTarget.value)
                    }
                  >
                    {models[key.id].map((model) => (
                      <option key={model.id} value={model.id}>
                        {model.displayName}
                      </option>
                    ))}
                  </select>
                ) : (
                  <button
                    type="button"
                    disabled={!isNative || loadingModels === key.id}
                    onClick={() => loadModels(key)}
                  >
                    {loadingModels === key.id
                      ? "加载模型…"
                      : key.agentModel ?? "选择模型"}
                  </button>
                )}
              </div>
              <div className="key-actions">
                <button
                  type="button"
                  disabled={!isNative || key.isAgentActive || busyId === key.id}
                  onClick={() => activate(key, "agent")}
                >
                  {key.isAgentActive ? "使用中" : "设为 Agent"}
                </button>
                <button
                  className="key-delete-button"
                  type="button"
                  disabled={!isNative || busyId === key.id}
                  onClick={() => remove(key)}
                >
                  删除
                </button>
              </div>
            </article>
          ))}
        </CredentialColumn>

        <CredentialColumn title="Embedding 凭据" count={embeddingKeys.length}>
          {embeddingKeys.length === 0 ? (
            <EmptyCredentials message="仅支持 OpenAI / Qwen" />
          ) : embeddingKeys.map((key) => (
            <article className="credential-card" key={key.id}>
              <CredentialHeading
                item={key}
                active={key.isEmbeddingActive}
                activeLabel="Embedding 使用中"
              />
              <p className="credential-hint">
                用于 Profile 构建、增量同步与向量迁移
              </p>
              <div className="key-actions">
                <button
                  type="button"
                  disabled={
                    !isNative || key.isEmbeddingActive || busyId === key.id
                  }
                  onClick={() => activate(key, "embedding")}
                >
                  {key.isEmbeddingActive ? "使用中" : "设为 Embedding"}
                </button>
              </div>
            </article>
          ))}
        </CredentialColumn>
      </div>
    </div>
  );
}

function CredentialColumn({
  title,
  count,
  children,
}: {
  title: string;
  count: number;
  children: React.ReactNode;
}) {
  return (
    <section className="credential-column">
      <header><h3>{title}</h3><span>{count}</span></header>
      <div className="credential-list">{children}</div>
    </section>
  );
}

function CredentialHeading({
  item,
  active,
  activeLabel,
}: {
  item: ApiKeyMetadata;
  active: boolean;
  activeLabel: string;
}) {
  return (
    <header>
      <div>
        <h4>{providerNames[item.provider]}</h4>
        <code>{item.maskedKey}</code>
      </div>
      {active && <span className="status-badge status-badge--active">{activeLabel}</span>}
    </header>
  );
}

function EmptyCredentials({ message = "尚未保存凭据" }: { message?: string }) {
  return <div className="empty-card">{message}</div>;
}
