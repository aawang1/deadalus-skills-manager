import { useEffect, useMemo, useRef, useState } from "react";
import type {
  CreateCustomSkillCategoryRequest,
  InstalledSkill,
} from "../types";

const categoryColors = [
  { value: "#4f8cff", label: "信号蓝" },
  { value: "#8b7cff", label: "星云紫" },
  { value: "#de6ea8", label: "莓果红" },
  { value: "#ef795f", label: "熔岩橙" },
  { value: "#e0b44f", label: "琥珀黄" },
  { value: "#57bd87", label: "终端绿" },
  { value: "#45b9c7", label: "电路青" },
  { value: "#a7b0bf", label: "合金灰" },
] as const;

interface CreateCategoryDialogProps {
  skills: InstalledSkill[];
  onClose: () => void;
  onCreate: (request: CreateCustomSkillCategoryRequest) => Promise<void>;
}

export function CreateCategoryDialog({
  skills,
  onClose,
  onCreate,
}: CreateCategoryDialogProps) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [color, setColor] = useState<string>();
  const [query, setQuery] = useState("");
  const [selectedSkillIds, setSelectedSkillIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const nameInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    requestAnimationFrame(() => nameInputRef.current?.focus());
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !saving) onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, saving]);

  const visibleSkills = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase();
    return [...skills]
      .filter(
        (skill) =>
          !skill.isBuiltIn &&
          (!normalizedQuery ||
            skill.name.toLocaleLowerCase().includes(normalizedQuery) ||
            skill.description?.toLocaleLowerCase().includes(normalizedQuery)),
      )
      .sort((left, right) => left.name.localeCompare(right.name));
  }, [query, skills]);

  const toggleSkill = (skillId: string) => {
    setSelectedSkillIds((current) => {
      const next = new Set(current);
      if (next.has(skillId)) next.delete(skillId);
      else next.add(skillId);
      return next;
    });
  };

  const complete = Boolean(
    name.trim() && description.trim() && color && selectedSkillIds.size > 0,
  );

  const submit = async () => {
    if (!complete || !color || saving) return;
    setSaving(true);
    setError("");
    try {
      await onCreate({
        name: name.trim(),
        color,
        description: description.trim(),
        skillIds: [...selectedSkillIds],
      });
    } catch (nextError) {
      setError(String(nextError));
      setSaving(false);
    }
  };

  return (
    <div className="dialog-backdrop" role="presentation">
      <section
        className="settings-dialog create-category-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="create-category-title"
      >
        <header className="settings-dialog__header">
          <div>
            <p className="eyebrow">SKILL COLLECTION</p>
            <h2 id="create-category-title">新建自定义类别</h2>
          </div>
          <button
            className="close-button"
            type="button"
            aria-label="关闭新建类别"
            disabled={saving}
            onClick={onClose}
          >
            ×
          </button>
        </header>

        <div className="create-category-dialog__body">
          <label className="category-field">
            <span>类别名称</span>
            <input
              ref={nameInputRef}
              value={name}
              maxLength={48}
              placeholder="例如：Web 视觉与实现"
              onChange={(event) => setName(event.currentTarget.value)}
            />
          </label>

          <fieldset className="category-color-field">
            <legend>标志颜色 · 双击选择</legend>
            <div className="category-color-options">
              {categoryColors.map((option) => (
                <button
                  key={option.value}
                  className={color === option.value ? "is-selected" : ""}
                  type="button"
                  aria-label={`选择${option.label}`}
                  aria-pressed={color === option.value}
                  title={`双击选择${option.label}`}
                  style={{ "--category-color": option.value } as React.CSSProperties}
                  onDoubleClick={() => setColor(option.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      setColor(option.value);
                    }
                  }}
                >
                  <span />
                </button>
              ))}
            </div>
          </fieldset>

          <label className="category-field category-field--description">
            <span>类别简介</span>
            <textarea
              value={description}
              maxLength={500}
              placeholder="说明这一组 Skills 的共同用途"
              onChange={(event) => setDescription(event.currentTarget.value)}
            />
          </label>

          <section className="category-skill-picker" aria-labelledby="category-skills-title">
            <header>
              <div>
                <span id="category-skills-title">选择 Skills · 已忽略内置 · 双击切换</span>
                <strong>{selectedSkillIds.size} 已选择</strong>
              </div>
              <label>
                <span className="sr-only">搜索可选 Skills</span>
                <input
                  type="search"
                  value={query}
                  placeholder="搜索名称或简介"
                  onChange={(event) => setQuery(event.currentTarget.value)}
                />
              </label>
            </header>
            <div className="category-skill-picker__list">
              {visibleSkills.length === 0 ? (
                <p className="category-skill-picker__empty">没有匹配的 Skills</p>
              ) : (
                visibleSkills.map((skill) => {
                  const selected = selectedSkillIds.has(skill.skillId);
                  return (
                    <button
                      key={skill.skillId}
                      className={selected ? "is-selected" : ""}
                      type="button"
                      aria-pressed={selected}
                      title={`双击${selected ? "取消选择" : "选择"} ${skill.name}`}
                      onDoubleClick={() => toggleSkill(skill.skillId)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          toggleSkill(skill.skillId);
                        }
                      }}
                    >
                      <span className="category-skill-picker__signal" />
                      <span>
                        <strong>{skill.name}</strong>
                        <small>{skill.description || "暂无简介"}</small>
                      </span>
                      <em>{selected ? "已选择" : "双击选择"}</em>
                    </button>
                  );
                })
              )}
            </div>
          </section>

          {error && <p className="create-category-dialog__error" role="alert">{error}</p>}
        </div>

        <footer className="create-category-dialog__footer">
          <span>
            {!complete
              ? "填写名称、颜色、简介，并至少选择一个 Skill"
              : `将使用 ${selectedSkillIds.size} 个 Skills 创建类别`}
          </span>
          <div>
            <button type="button" disabled={saving} onClick={onClose}>取消</button>
            <button
              className="send-button"
              type="button"
              disabled={!complete || saving}
              onClick={submit}
            >
              {saving ? "正在创建…" : "创建类别"}
            </button>
          </div>
        </footer>
      </section>
    </div>
  );
}
