import { tr } from "../i18n";
import { DisplaySummary } from "./DisplaySummary";
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { api } from "../api";
import type { CreateCustomSkillCategoryRequest, InstalledSkill, } from "../types";
const categoryColors = [
    { value: "#4f8cff", get label() {
            return tr("信号蓝");
        } },
    { value: "#8b7cff", get label() {
            return tr("星云紫");
        } },
    { value: "#de6ea8", get label() {
            return tr("莓果红");
        } },
    { value: "#ef795f", get label() {
            return tr("熔岩橙");
        } },
    { value: "#e0b44f", get label() {
            return tr("琥珀黄");
        } },
    { value: "#57bd87", get label() {
            return tr("终端绿");
        } },
    { value: "#45b9c7", get label() {
            return tr("电路青");
        } },
    { value: "#a7b0bf", get label() {
            return tr("合金灰");
        } },
] as const;
interface CreateCategoryDialogProps {
    project?: boolean;
    skills: InstalledSkill[];
    fixedSkillIds?: string[];
    onClose: () => void;
    onCreate: (request: CreateCustomSkillCategoryRequest) => Promise<void>;
}
export function CreateCategoryDialog({ project = false, skills, fixedSkillIds, onClose, onCreate, }: CreateCategoryDialogProps) {
    const [name, setName] = useState("");
    const [projectRoot, setProjectRoot] = useState("");
    const [choosingDirectory, setChoosingDirectory] = useState(false);
    const [description, setDescription] = useState("");
    const [color, setColor] = useState<string>();
    const [query, setQuery] = useState("");
    const [selectedSkillIds, setSelectedSkillIds] = useState<Set<string>>(() => new Set(fixedSkillIds ?? []));
    const [saving, setSaving] = useState(false);
    const [error, setError] = useState("");
    const nameInputRef = useRef<HTMLInputElement>(null);
    useEffect(() => {
        requestAnimationFrame(() => nameInputRef.current?.focus());
        const onKeyDown = (event: KeyboardEvent) => {
            if (event.key === "Escape" && !saving)
                onClose();
        };
        window.addEventListener("keydown", onKeyDown);
        return () => window.removeEventListener("keydown", onKeyDown);
    }, [onClose, saving]);
    const visibleSkills = useMemo(() => {
        const normalizedQuery = query.trim().toLocaleLowerCase();
        return [...skills]
            .filter((skill) => !skill.isBuiltIn &&
            (!normalizedQuery ||
                skill.name.toLocaleLowerCase().includes(normalizedQuery) ||
                skill.description?.toLocaleLowerCase().includes(normalizedQuery)))
            .sort((left, right) => left.name.localeCompare(right.name));
    }, [query, skills]);
    const toggleSkill = (skillId: string) => {
        setSelectedSkillIds((current) => {
            const next = new Set(current);
            if (next.has(skillId))
                next.delete(skillId);
            else
                next.add(skillId);
            return next;
        });
    };
    const complete = Boolean(name.trim() && description.trim() && color && (project ? projectRoot : selectedSkillIds.size > 0));
    const submit = async () => {
        if (!complete || !color || saving)
            return;
        setSaving(true);
        setError("");
        try {
            await onCreate({
                name: name.trim(),
                color,
                description: description.trim(),
                skillIds: [...selectedSkillIds],
                ...(project ? { projectRoot } : {}),
            });
        }
        catch (nextError) {
            setError(String(nextError));
            setSaving(false);
        }
    };
    return createPortal(<div className="dialog-backdrop" role="presentation">
      <section className="settings-dialog create-category-dialog" role="dialog" aria-modal="true" aria-labelledby="create-category-title">
        <header className="settings-dialog__header">
          <div>
            <p className="eyebrow">{tr("Skill 集合")}</p>
            <h2 id="create-category-title">{project ? tr("新建项目") : tr("新建自定义类别")}</h2>
          </div>
          <button className="close-button" type="button" aria-label={tr("关闭新建类别")} disabled={saving} onClick={onClose}>
            ×
          </button>
        </header>

        <div className="create-category-dialog__body">
          <label className="category-field">
            <span>{tr("类别名称")}</span>
            <input ref={nameInputRef} value={name} maxLength={48} placeholder={tr("例如：Web 视觉与实现")} onChange={(event) => setName(event.currentTarget.value)}/>
          </label>

          <fieldset className="category-color-field">
            <legend>{tr("标志颜色 · 双击选择")}</legend>
            <div className="category-color-options">
              {categoryColors.map((option) => (<button key={option.value} className={color === option.value ? "is-selected" : ""} type="button" aria-label={tr("选择{0}", option.label)} aria-pressed={color === option.value} title={tr("双击选择{0}", option.label)} style={{ "--category-color": option.value } as React.CSSProperties} onDoubleClick={() => setColor(option.value)} onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                    event.preventDefault();
                    setColor(option.value);
                }
            }}>
                  <span />
                </button>))}
            </div>
          </fieldset>

          <label className="category-field category-field--description">
            <span>{tr("类别简介")}</span>
            <textarea value={description} maxLength={500} placeholder={tr("说明这一组 Skills 的共同用途")} onChange={(event) => setDescription(event.currentTarget.value)}/>
          </label>

          {project ? (<section className="project-directory-picker">
              <button className="project-directory-picker__button" type="button" aria-describedby="project-directory-help" disabled={choosingDirectory || saving} onClick={async () => {
                setChoosingDirectory(true);
                setError("");
                try {
                    const directory = await api.selectProjectDirectory();
                    if (directory)
                        setProjectRoot(directory);
                }
                catch (nextError) {
                    setError(String(nextError));
                }
                finally {
                    setChoosingDirectory(false);
                }
            }}>{choosingDirectory ? tr("正在选择…") : tr("选择项目根目录文件夹")}</button>
              {projectRoot && <p className="credential-hint project-directory-picker__path" role="status">{projectRoot}</p>}
              <p className="credential-hint" id="project-directory-help">{tr("创建和复制时同步补齐项目 .agents/skills 与 .claude/skills，供 Codex、Cursor 和 Claude Code 使用。同名不同内容时会提示冲突。")}</p>
            </section>) : fixedSkillIds ? (<p className="category-skill-picker__empty">{tr("将使用推荐的")}{fixedSkillIds.length}{tr("个 Skills；只需填写名称、颜色和简介。")}</p>) : <section className="category-skill-picker" aria-labelledby="category-skills-title">
            <header>
              <div>
                <span id="category-skills-title">{tr("选择 Skills · 已忽略内置 · 双击切换")}</span>
                <strong>{tr("{0} 已选择", selectedSkillIds.size)}</strong>
              </div>
              <label>
                <span className="sr-only">{tr("搜索可选 Skills")}</span>
                <input type="search" value={query} placeholder={tr("搜索名称或简介")} onChange={(event) => setQuery(event.currentTarget.value)}/>
              </label>
            </header>
            <div className="category-skill-picker__list">
              {visibleSkills.length === 0 ? (<p className="category-skill-picker__empty">{tr("没有匹配的 Skills")}</p>) : (visibleSkills.map((skill) => {
                const selected = selectedSkillIds.has(skill.skillId);
                return (<button key={skill.skillId} className={selected ? "is-selected" : ""} type="button" aria-pressed={selected} title={tr("双击{0} {1}", selected ? tr("取消选择") : tr("选择"), skill.name)} onDoubleClick={() => toggleSkill(skill.skillId)} onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                            event.preventDefault();
                            toggleSkill(skill.skillId);
                        }
                    }}>
                      <span className="category-skill-picker__signal"/>
                      <span>
                        <strong>{skill.name}</strong>
                        <small><DisplaySummary text={skill.description}/></small>
                      </span>
                      <em>{selected ? tr("已选择") : tr("双击选择")}</em>
                    </button>);
            }))}
            </div>
          </section>}

          {error && <p className="create-category-dialog__error" role="alert">{error}</p>}
        </div>

        <footer className="create-category-dialog__footer">
          <span>
            {project ? (complete ? tr("确认后绑定项目并读取项目专用 Skills") : tr("填写名称、颜色、简介，并选择项目根目录")) : !complete
            ? tr("填写名称、颜色、简介，并至少选择一个 Skill") : tr("将使用 {0} 个 Skills 创建类别", selectedSkillIds.size)}
          </span>
          <div>
            <button type="button" disabled={saving} onClick={onClose}>{tr("取消")}</button>
            <button className="send-button" type="button" disabled={!complete || saving} onClick={submit}>
              {saving ? tr("正在创建…") : tr("创建类别")}
            </button>
          </div>
        </footer>
      </section>
    </div>, document.body);
}
