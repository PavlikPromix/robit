import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  ArchiveRestore,
  CheckCircle2,
  FolderInput,
  FolderOpen,
  FolderOutput,
  History,
  Link2,
  Loader2,
  Maximize2,
  Minus,
  Play,
  Power,
  RotateCcw,
  ShieldAlert,
  ShieldCheck,
  Square,
  X,
  XCircle
} from "lucide-react";
import { type MouseEvent, useCallback, useEffect, useMemo, useState } from "react";

type MoveStrategy = "safe_copy_delete" | "robocopy_move";
type ItemKind = "file" | "directory";
type OperationStatus =
  | "planned"
  | "copying"
  | "copied"
  | "deleting_source"
  | "linking"
  | "completed"
  | "cancelled"
  | "failed"
  | "rolling_back"
  | "rolled_back";

type FileLock = {
  path: string;
  pid: number;
  process_name: string;
  application_name: string;
};

type MovePreview = {
  source_path: string;
  destination_parent: string;
  destination_path: string;
  item_kind: ItemKind;
  locks: FileLock[];
};

type OperationSnapshot = {
  id: string;
  source_path: string;
  destination_parent: string;
  destination_path: string;
  item_kind: ItemKind;
  strategy: MoveStrategy;
  status: OperationStatus;
  created_at: string;
  updated_at: string;
  log_path: string;
  error_message?: string | null;
  progress_current?: number | null;
  progress_total?: number | null;
  progress_label?: string | null;
};

type ProgressSnapshot = {
  current: number;
  total: number;
  label: string;
};

type PreviewCheck = {
  key: string;
  checkedAt: number;
  hasLocks: boolean;
};

type LogRead = {
  lines: string[];
  next_offset: number;
};

const PREVIEW_REUSE_MS = 60_000;
const APP_SETTINGS_KEY = "robit-link-mover-settings";

type AppSettings = {
  sourcePath: string;
  destinationParent: string;
  checkLocksBeforeMove: boolean;
};

const statusLabels: Record<OperationStatus, string> = {
  planned: "Запланировано",
  copying: "Копирование",
  copied: "Скопировано",
  deleting_source: "Удаление исходника",
  linking: "Создание ссылки",
  completed: "Завершено",
  cancelled: "Отменено",
  failed: "Ошибка",
  rolling_back: "Откат",
  rolled_back: "Откат выполнен"
};

function formatPath(path: string) {
  return path || "Не выбрано";
}

function loadSettings(): AppSettings {
  try {
    const raw = window.localStorage.getItem(APP_SETTINGS_KEY);
    if (!raw) {
      return { sourcePath: "", destinationParent: "", checkLocksBeforeMove: true };
    }
    const parsed = JSON.parse(raw) as Partial<AppSettings>;
    return {
      sourcePath: typeof parsed.sourcePath === "string" ? parsed.sourcePath : "",
      destinationParent: typeof parsed.destinationParent === "string" ? parsed.destinationParent : "",
      checkLocksBeforeMove:
        typeof parsed.checkLocksBeforeMove === "boolean" ? parsed.checkLocksBeforeMove : true
    };
  } catch {
    return { sourcePath: "", destinationParent: "", checkLocksBeforeMove: true };
  }
}

function canRollback(op: OperationSnapshot) {
  return op.status === "completed";
}

function canCancel(op?: OperationSnapshot | null) {
  return Boolean(op && ["planned", "copying", "copied", "deleting_source", "linking"].includes(op.status));
}

function progressPercent(current?: number | null, total?: number | null) {
  if (!total || total <= 0) return 100;
  return Math.max(0, Math.min(100, Math.round(((current ?? 0) / total) * 100)));
}

function formatProgress(current?: number | null, total?: number | null) {
  if (!total || total <= 0) return "Готово";
  return `${current ?? 0} из ${total}`;
}

function formatDuration(ms: number) {
  if (!Number.isFinite(ms) || ms <= 0) return "меньше минуты";
  const totalSeconds = Math.ceil(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes <= 0) return `${seconds} сек.`;
  if (minutes < 60) return seconds > 0 ? `${minutes} мин. ${seconds} сек.` : `${minutes} мин.`;
  const hours = Math.floor(minutes / 60);
  const restMinutes = minutes % 60;
  return restMinutes > 0 ? `${hours} ч. ${restMinutes} мин.` : `${hours} ч.`;
}

function estimateRemaining(op: OperationSnapshot) {
  const current = op.progress_current ?? 0;
  const total = op.progress_total ?? 0;
  if (total <= 0 || current <= 0 || current >= total) return null;
  const elapsed = Date.now() - new Date(op.created_at).getTime();
  const remaining = (elapsed / current) * (total - current);
  return formatDuration(remaining);
}

function previewKey(sourcePath: string, destinationParent: string, strategy: MoveStrategy) {
  return `${sourcePath}\n${destinationParent}\n${strategy}`;
}

function currentWindow() {
  return getCurrentWindow();
}

export default function App() {
  const [settingsLoaded] = useState(loadSettings);
  const [sourcePath, setSourcePath] = useState(settingsLoaded.sourcePath);
  const [destinationParent, setDestinationParent] = useState(settingsLoaded.destinationParent);
  const [strategy, setStrategy] = useState<MoveStrategy>("safe_copy_delete");
  const [preview, setPreview] = useState<MovePreview | null>(null);
  const [operations, setOperations] = useState<OperationSnapshot[]>([]);
  const [activeOperation, setActiveOperation] = useState<OperationSnapshot | null>(null);
  const [logLines, setLogLines] = useState<string[]>([]);
  const [logOffset, setLogOffset] = useState(0);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [elevated, setElevated] = useState(false);
  const [checkLocksBeforeMove, setCheckLocksBeforeMove] = useState(settingsLoaded.checkLocksBeforeMove);
  const [previewProgress, setPreviewProgress] = useState<ProgressSnapshot | null>(null);
  const [previewCheck, setPreviewCheck] = useState<PreviewCheck | null>(null);

  const activeId = activeOperation?.id;

  const refreshOperations = useCallback(async () => {
    const items = await invoke<OperationSnapshot[]>("list_operations");
    setOperations(items);
    if (activeId) {
      const updated = items.find((item) => item.id === activeId);
      if (updated) setActiveOperation(updated);
    }
  }, [activeId]);

  useEffect(() => {
    refreshOperations().catch((error) => setMessage(String(error)));
    invoke<boolean>("is_elevated")
      .then(setElevated)
      .catch(() => setElevated(false));
  }, [refreshOperations]);

  useEffect(() => {
    window.localStorage.setItem(
      APP_SETTINGS_KEY,
      JSON.stringify({ sourcePath, destinationParent, checkLocksBeforeMove })
    );
  }, [sourcePath, destinationParent, checkLocksBeforeMove]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      refreshOperations().catch(() => undefined);
    }, 1500);
    return () => window.clearInterval(timer);
  }, [refreshOperations]);

  useEffect(() => {
    if (!activeId) return;
    const timer = window.setInterval(async () => {
      const chunk = await invoke<LogRead>("read_operation_log", { id: activeId, offset: logOffset });
      if (chunk.lines.length > 0) {
        setLogLines((current) => [...current, ...chunk.lines].slice(-500));
        setLogOffset(chunk.next_offset);
      }
    }, 900);
    return () => window.clearInterval(timer);
  }, [activeId, logOffset]);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let disposed = false;

    listen<ProgressSnapshot>("preview-progress", (event) => {
      if (!disposed) setPreviewProgress(event.payload);
    }).then((handler) => {
      if (disposed) {
        handler();
      } else {
        unlisten = handler;
      }
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const selectedOperation = useMemo(() => activeOperation ?? operations[0] ?? null, [activeOperation, operations]);
  const selectedRemaining = selectedOperation ? estimateRemaining(selectedOperation) : null;

  function clearPreviewState() {
    setPreview(null);
    setPreviewProgress(null);
    setPreviewCheck(null);
  }

  function updateSourcePath(value: string) {
    setSourcePath(value);
    clearPreviewState();
  }

  function updateDestinationParent(value: string) {
    setDestinationParent(value);
    clearPreviewState();
  }

  async function pickSource() {
    const selected = await open({
      multiple: false,
      directory: false
    });
    if (typeof selected === "string") {
      updateSourcePath(selected);
    }
  }

  async function pickSourceFolder() {
    const selected = await open({
      multiple: false,
      directory: true
    });
    if (typeof selected === "string") {
      updateSourcePath(selected);
    }
  }

  async function pickDestination() {
    const selected = await open({
      multiple: false,
      directory: true
    });
    if (typeof selected === "string") {
      updateDestinationParent(selected);
    }
  }

  function changeStrategy(value: MoveStrategy) {
    setStrategy(value);
    clearPreviewState();
  }

  async function openPath(path: string) {
    if (!path) return;
    try {
      await invoke("open_path_in_explorer", { path });
    } catch (error) {
      setMessage(String(error));
    }
  }

  function selectOperation(op: OperationSnapshot) {
    setActiveOperation(op);
    setSourcePath(op.source_path);
    setDestinationParent(op.destination_parent);
    clearPreviewState();
    setLogLines([]);
    setLogOffset(0);
  }

  async function buildPreview() {
    setBusy(true);
    setMessage(null);
    setPreviewProgress({ current: 0, total: 1, label: "Подготовка проверки" });
    try {
      const result = await invoke<MovePreview>("preview_move", {
        request: {
          source_path: sourcePath,
          destination_parent: destinationParent,
          strategy
        }
      });
      setPreview(result);
      setPreviewCheck({
        key: previewKey(sourcePath, destinationParent, strategy),
        checkedAt: Date.now(),
        hasLocks: result.locks.length > 0
      });
      setPreviewProgress((current) => ({
        current: current?.total ?? 0,
        total: current?.total ?? 0,
        label: result.locks.length === 0 ? "Проверка завершена" : "Проверка завершена, есть блокировки"
      }));
    } catch (error) {
      setPreviewCheck(null);
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function startMove() {
    setBusy(true);
    setMessage(null);
    setLogLines([]);
    setLogOffset(0);
    const canReusePreview =
      checkLocksBeforeMove &&
      previewCheck !== null &&
      previewCheck.key === previewKey(sourcePath, destinationParent, strategy) &&
      !previewCheck.hasLocks &&
      Date.now() - previewCheck.checkedAt < PREVIEW_REUSE_MS;
    const skipLockCheck = !checkLocksBeforeMove || canReusePreview;
    setPreviewProgress({
      current: 0,
      total: 1,
      label: !checkLocksBeforeMove
        ? "Запуск без проверки блокировок"
        : canReusePreview
          ? "Запуск без повторной проверки"
          : "Проверка перед запуском"
    });
    try {
      const result = await invoke<OperationSnapshot>("start_move", {
        request: {
          source_path: sourcePath,
          destination_parent: destinationParent,
          strategy,
          skip_lock_check: skipLockCheck
        }
      });
      setActiveOperation(result);
      setPreviewProgress(null);
      await refreshOperations();
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function cancelMove() {
    if (!activeOperation) return;
    setBusy(true);
    try {
      await invoke("cancel_operation", { id: activeOperation.id });
      await refreshOperations();
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function rollback(id: string) {
    setBusy(true);
    setMessage(null);
    setLogLines([]);
    setLogOffset(0);
    try {
      const result = await invoke<OperationSnapshot>("rollback_operation", { id });
      setActiveOperation(result);
      await refreshOperations();
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function restartAsAdmin() {
    setBusy(true);
    setMessage(null);
    try {
      await invoke("restart_as_admin");
    } catch (error) {
      setMessage(String(error));
      setBusy(false);
    }
  }

  function startWindowDrag(event: MouseEvent<HTMLElement>) {
    if (event.buttons !== 1) return;
    const target = event.target as HTMLElement;
    if (target.closest("button")) return;
    currentWindow().startDragging().catch(() => undefined);
  }

  function handleTitlebarDoubleClick(event: MouseEvent<HTMLElement>) {
    const target = event.target as HTMLElement;
    if (target.closest("button")) return;
    toggleWindowMaximize();
  }

  function toggleWindowMaximize() {
    currentWindow().toggleMaximize().catch(() => undefined);
  }

  function minimizeWindow() {
    currentWindow().minimize().catch(() => undefined);
  }

  function closeWindow() {
    currentWindow().close().catch(() => undefined);
  }

  return (
    <main className="window-shell">
      <section className="app-window">
        <div className="workspace">
        <header className="titlebar" onMouseDown={startWindowDrag} onDoubleClick={handleTitlebarDoubleClick}>
          <div className="brand-block">
            <img className="app-logo" src="/logo.png" alt="" />
            <div>
              <h1>Robit Link Mover</h1>
              <p>Перенос файлов и папок с сохранением старого пути через ссылку.</p>
            </div>
          </div>
          <div className="window-side">
            <div className={elevated ? "status-pill elevated" : "status-pill"}>
              {elevated ? <ShieldCheck size={16} /> : <ShieldAlert size={16} />}
              {elevated ? "Администратор" : "Обычный режим"}
            </div>
            <div className="window-controls" aria-label="Управление окном">
              <button type="button" className="window-button" onClick={minimizeWindow} title="Свернуть">
                <Minus size={18} />
              </button>
              <button type="button" className="window-button" onClick={toggleWindowMaximize} title="Развернуть">
                <Maximize2 size={16} />
              </button>
              <button type="button" className="window-button close" onClick={closeWindow} title="Закрыть">
                <X size={18} />
              </button>
            </div>
          </div>
        </header>
		<div className="workspace-scroll">
			<div className="layout">
			  <section className="panel mover-panel">
				<div className="field-grid">
				  <label>
					<span>Источник</span>
					<div className="path-row">
					  <input
						className="path-input"
						value={sourcePath}
						onChange={(event) => updateSourcePath(event.target.value)}
						placeholder="C:\\путь\\к\\файлу или папке"
						title={sourcePath}
						spellCheck={false}
					  />
					  <button type="button" className="icon-button" onClick={pickSource} title="Выбрать файл">
						<FolderInput size={18} />
					  </button>
					  <button type="button" className="icon-button" onClick={pickSourceFolder} title="Выбрать папку">
						<ArchiveRestore size={18} />
					  </button>
					</div>
				  </label>

				  <label>
					<span>Папка назначения</span>
					<div className="path-row">
					  <input
						className="path-input"
						value={destinationParent}
						onChange={(event) => updateDestinationParent(event.target.value)}
						placeholder="C:\\папка\\назначения"
						title={destinationParent}
						spellCheck={false}
					  />
					  <button type="button" className="icon-button" onClick={pickDestination} title="Выбрать папку назначения">
						<FolderOutput size={18} />
					  </button>
					</div>
				  </label>
				</div>

				<div className="strategy-row" role="radiogroup" aria-label="Режим переноса">
				  <button
					type="button"
					className={strategy === "safe_copy_delete" ? "segment active" : "segment"}
					onClick={() => changeStrategy("safe_copy_delete")}
				  >
					Безопасный copy-delete
				  </button>
				  <button
					type="button"
					className={strategy === "robocopy_move" ? "segment active danger" : "segment"}
					onClick={() => changeStrategy("robocopy_move")}
				  >
					Robocopy /MOVE
				  </button>
				</div>

				{strategy === "robocopy_move" && (
				  <div className="notice">
					<AlertTriangle size={18} />
					Прямой `/MOVE` быстрее для больших папок, но отмена во время копирования менее надежна.
				  </div>
				)}

				<label className="option-row">
				  <input
					type="checkbox"
					checked={checkLocksBeforeMove}
					onChange={(event) => setCheckLocksBeforeMove(event.target.checked)}
				  />
				  <ShieldCheck size={18} />
				  <span>
					<strong>Проверять блокировки перед переносом</strong>
					<small>
					  Если отключить, перенос стартует без предварительной проверки занятых файлов.
					</small>
				  </span>
				</label>

				{!elevated && (
				  <div className="admin-box">
					<ShieldAlert size={18} />
					<span>Для нескольких переносов без повторного UAC перезапустите приложение от администратора.</span>
					<button type="button" className="secondary" disabled={busy} onClick={restartAsAdmin}>
					  <Power size={16} />
					  Перезапустить
					</button>
				  </div>
				)}

				<div className="actions">
				  <button type="button" className="secondary" disabled={!sourcePath || !destinationParent || busy} onClick={buildPreview}>
					Проверить
				  </button>
				  <button type="button" className="primary" disabled={!sourcePath || !destinationParent || busy} onClick={startMove}>
					{busy ? <Loader2 className="spin" size={18} /> : <Play size={18} />}
					Запустить
				  </button>
				  <button type="button" className="danger-button" disabled={!canCancel(activeOperation) || busy} onClick={cancelMove}>
					<Square size={16} />
					Отменить
				  </button>
				</div>

				{message && (
				  <div className="error-line">
					<XCircle size={18} />
					{message}
				  </div>
				)}

				{previewProgress && (
				  <section className="progress-box" aria-live="polite">
					<div className="progress-head">
					  <span>{previewProgress.label}</span>
					  <strong>{progressPercent(previewProgress.current, previewProgress.total)}%</strong>
					</div>
					<div className="progress-track">
					  <span style={{ width: `${progressPercent(previewProgress.current, previewProgress.total)}%` }} />
					</div>
					<div className="progress-meta">
					  <span>{formatProgress(previewProgress.current, previewProgress.total)}</span>
					  {previewProgress.total >= 2000 && <span>Показан лимит глубокой проверки</span>}
					</div>
				  </section>
				)}

				{preview && (
				  <section className="preview-box">
					<div>
					  <span>Будет создан путь</span>
					  <code title={preview.destination_path}>{preview.destination_path}</code>
					</div>
					<div>
					  <span>Тип</span>
					  <strong>{preview.item_kind === "directory" ? "Папка / junction" : "Файл / symlink"}</strong>
					</div>
					<div>
					  <span>Блокировки</span>
					  <strong>{preview.locks.length === 0 ? "Не найдены" : `${preview.locks.length} найдено`}</strong>
					</div>
				  </section>
				)}

				{preview && preview.locks.length > 0 && (
				  <section className="locks-box">
					<h2>Файлы заняты процессами</h2>
					{preview.locks.map((lock) => (
					  <div className="lock-row" key={`${lock.path}-${lock.pid}`}>
						<code>{lock.path}</code>
						<span>{lock.process_name || lock.application_name}</span>
						<strong>PID {lock.pid}</strong>
					  </div>
					))}
				  </section>
				)}
			  </section>

			  <section className="panel side-panel">
				<div className="panel-title">
				  <History size={18} />
				  История
				</div>
				<div className="operation-list">
				  {operations.length === 0 && <p className="muted">Операций пока нет.</p>}
				  {operations.map((op) => (
					<button
					  type="button"
					  key={op.id}
					  className={selectedOperation?.id === op.id ? "operation active" : "operation"}
					  onClick={() => selectOperation(op)}
					>
					  <span className={`dot ${op.status}`} />
					  <span>
						<strong>{statusLabels[op.status]}</strong>
						<small title={`${op.source_path} → ${op.destination_parent}`}>
						  {op.source_path} → {op.destination_parent}
						</small>
					  </span>
					</button>
				  ))}
				</div>
			  </section>
			</div>

			<section className="panel log-panel">
			  <div className="panel-title">
				{selectedOperation?.status === "completed" ? <CheckCircle2 size={18} /> : <Link2 size={18} />}
				Журнал операции
				{selectedOperation && (
				  <button
					type="button"
					className="secondary small"
					disabled={!canRollback(selectedOperation) || busy}
					onClick={() => rollback(selectedOperation.id)}
				  >
					<RotateCcw size={16} />
					Откатить
				  </button>
				)}
			  </div>
			  {selectedOperation && (
				<div className="operation-summary">
				  <button
					type="button"
					className="path-link"
					onClick={() => openPath(selectedOperation.source_path)}
					title="Открыть источник в проводнике"
				  >
					<FolderOpen size={15} />
					<code>{formatPath(selectedOperation.source_path)}</code>
				  </button>
				  <span>→</span>
				  <button
					type="button"
					className="path-link"
					onClick={() => openPath(selectedOperation.destination_parent)}
					title="Открыть папку назначения в проводнике"
				  >
					<FolderOpen size={15} />
					<code>{formatPath(selectedOperation.destination_parent)}</code>
				  </button>
				</div>
			  )}
			  {selectedOperation && selectedOperation.progress_total != null && (
				<section className="progress-box operation-progress" aria-live="polite">
				  <div className="progress-head">
					<span>{selectedOperation.progress_label || statusLabels[selectedOperation.status]}</span>
					<strong>{progressPercent(selectedOperation.progress_current, selectedOperation.progress_total)}%</strong>
				  </div>
				  <div className="progress-track">
					<span
					  style={{
						width: `${progressPercent(selectedOperation.progress_current, selectedOperation.progress_total)}%`
					  }}
					/>
				  </div>
				  <div className="progress-meta">
					<span>{formatProgress(selectedOperation.progress_current, selectedOperation.progress_total)}</span>
					{selectedRemaining && <span>Осталось примерно {selectedRemaining}</span>}
				  </div>
				</section>
			  )}
			  <pre className="log-output">
				{logLines.length === 0 ? "Логи появятся после запуска операции." : logLines.join("\n")}
			  </pre>
			</section>
			</div>
		</div>
      </section>
    </main>
  );
}
