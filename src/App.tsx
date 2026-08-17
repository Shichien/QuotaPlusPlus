import {
  Check,
  CircleUserRound,
  LoaderCircle,
  LogIn,
  Plus,
  ShieldCheck,
  X,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { ProviderCard } from "./components/ProviderCard";
import { ProviderDialog } from "./components/ProviderDialog";
import { cswitchApi } from "./lib/api";
import type {
  ProviderDraft,
  ProviderState,
  ProviderSummary,
  SavedProvider,
} from "./types";

const EMPTY_STATE: ProviderState = {
  providers: [],
  activeProviderId: null,
  officialActive: false,
};

type Notice = { kind: "success" | "error"; text: string } | null;
type BusyAction = "load" | "official" | "save" | "route" | "delete" | string | null;

function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

function App() {
  const [state, setState] = useState<ProviderState>(EMPTY_STATE);
  const [busy, setBusy] = useState<BusyAction>("load");
  const [notice, setNotice] = useState<Notice>(null);
  const [providerDialogOpen, setProviderDialogOpen] = useState(false);
  const [editingProvider, setEditingProvider] = useState<ProviderSummary | null>(null);
  const [routingProvider, setRoutingProvider] = useState<ProviderSummary | null>(null);
  const [deletingProvider, setDeletingProvider] = useState<ProviderSummary | null>(null);
  const noticeTimer = useRef<number | null>(null);

  const showNotice = useCallback((kind: "success" | "error", text: string) => {
    if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
    setNotice({ kind, text });
    noticeTimer.current = window.setTimeout(() => setNotice(null), 3000);
  }, []);

  const refresh = useCallback(async () => {
    setState(await cswitchApi.listProviders());
  }, []);

  useEffect(() => {
    let active = true;
    cswitchApi
      .listProviders()
      .then((next) => {
        if (active) setState(next);
      })
      .catch((error) => {
        if (active) showNotice("error", errorText(error));
      })
      .finally(() => {
        if (active) setBusy(null);
      });
    return () => {
      active = false;
      if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
    };
  }, [showNotice]);

  const openAdd = () => {
    setEditingProvider(null);
    setProviderDialogOpen(true);
  };

  const openEdit = (provider: ProviderSummary) => {
    setEditingProvider(provider);
    setProviderDialogOpen(true);
  };

  const activate = async (provider: ProviderSummary) => {
    if (provider.protocol !== "openai_responses" && provider.routingMode !== "local") {
      setRoutingProvider(provider);
      return;
    }
    setBusy(provider.id);
    try {
      const report = await cswitchApi.activateProvider(provider.id);
      await refresh();
      showNotice("success", `已切换到 ${provider.name}，已同步 ${report.rolloutFilesUpdated} 个任务`);
    } catch (error) {
      showNotice("error", errorText(error));
    } finally {
      setBusy(null);
    }
  };

  const save = async (draft: ProviderDraft) => {
    const wasActive = Boolean(editingProvider?.active);
    setBusy("save");
    try {
      const result: SavedProvider = await cswitchApi.saveProvider(editingProvider?.id ?? null, draft);
      setProviderDialogOpen(false);
      setEditingProvider(null);
      await refresh();
      if (result.routingRequired) {
        setRoutingProvider(result.provider);
        showNotice("success", "供应商已保存");
        return;
      }
      if (wasActive) {
        await cswitchApi.activateProvider(result.provider.id);
        await refresh();
      }
      showNotice("success", wasActive ? "供应商已更新并重新应用" : "供应商已保存");
    } catch (error) {
      showNotice("error", errorText(error));
    } finally {
      setBusy(null);
    }
  };

  const enableRouting = async () => {
    if (!routingProvider) return;
    const provider = routingProvider;
    setBusy("route");
    try {
      await cswitchApi.enableProviderRouting(provider.id);
      const report = await cswitchApi.activateProvider(provider.id);
      setRoutingProvider(null);
      await refresh();
      showNotice("success", `已切换到 ${provider.name}，已同步 ${report.rolloutFilesUpdated} 个任务`);
    } catch (error) {
      showNotice("error", errorText(error));
    } finally {
      setBusy(null);
    }
  };

  const removeProvider = async () => {
    if (!deletingProvider) return;
    setBusy("delete");
    try {
      await cswitchApi.deleteProvider(deletingProvider.id);
      setDeletingProvider(null);
      await refresh();
      showNotice("success", "供应商已删除");
    } catch (error) {
      showNotice("error", errorText(error));
    } finally {
      setBusy(null);
    }
  };

  const useOfficial = async () => {
    if (busy) return;
    setBusy("official");
    try {
      const report = await cswitchApi.startOfficialLogin();
      await refresh();
      showNotice("success", `已切换到官方登录，已同步 ${report.rolloutFilesUpdated} 个任务`);
    } catch (error) {
      const message = errorText(error);
      if (message !== "官方登录已取消") showNotice("error", message);
    } finally {
      setBusy(null);
    }
  };

  const cancelOfficial = async () => {
    try {
      await cswitchApi.cancelOfficialLogin();
      showNotice("success", "已取消官方登录");
    } catch (error) {
      showNotice("error", errorText(error));
    }
  };

  const isLoading = busy === "load";

  return (
    <div className="app-shell">
      {notice && (
        <div className={`notice ${notice.kind}`} role={notice.kind === "error" ? "alert" : "status"}>
          {notice.kind === "error" ? <X size={16} /> : <Check size={16} />}
          <span>{notice.text}</span>
        </div>
      )}

      <header className="topbar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">
            <ShieldCheck size={21} />
          </span>
          <h1>CSwitch</h1>
        </div>
        <button className="add-button" type="button" aria-label="添加供应商" title="添加供应商" disabled={Boolean(busy)} onClick={openAdd}>
          <Plus size={19} />
        </button>
      </header>

      <main>
        <section aria-label="官方登录">
          <article className={`provider-card official-card${state.officialActive ? " active" : ""}`}>
            <button className="provider-select" type="button" disabled={Boolean(busy) || state.officialActive} onClick={useOfficial}>
              <span className="provider-icon official-icon" aria-hidden="true">
                <CircleUserRound size={20} />
              </span>
              <span className="provider-copy">
                <span className="provider-title-row">
                  <strong>官方登录</strong>
                  {state.officialActive && (
                    <span className="active-badge"><Check size={12} /> 当前</span>
                  )}
                </span>
                <span className="provider-meta"><span>ChatGPT OAuth</span></span>
              </span>
              {busy === "official" ? <LoaderCircle className="spinner" size={18} /> : <LogIn size={18} />}
            </button>
            {busy === "official" && (
              <div className="provider-actions">
                <button className="cancel-login-button" type="button" onClick={cancelOfficial}>取消</button>
              </div>
            )}
          </article>
        </section>

        <div className="section-heading">
          <h2>API 供应商</h2>
          <span className="count-badge">{state.providers.length}</span>
        </div>

        <section className="provider-list" aria-label="API 供应商列表" aria-busy={isLoading}>
          {isLoading && <div className="loading-row"><LoaderCircle className="spinner" size={22} /></div>}
          {!isLoading && state.providers.length === 0 && (
            <div className="empty-state">
              <Plus size={21} />
              <button type="button" onClick={openAdd}>添加第一个供应商</button>
            </div>
          )}
          {state.providers.map((provider) => (
            <ProviderCard
              key={provider.id}
              provider={provider}
              disabled={Boolean(busy)}
              activating={busy === provider.id}
              onActivate={() => void activate(provider)}
              onEdit={() => openEdit(provider)}
              onDelete={() => setDeletingProvider(provider)}
              onEnableRouting={() => setRoutingProvider(provider)}
            />
          ))}
        </section>
      </main>

      {providerDialogOpen && (
        <ProviderDialog
          provider={editingProvider}
          busy={busy === "save"}
          onCancel={() => {
            if (busy !== "save") setProviderDialogOpen(false);
          }}
          onSubmit={(draft) => void save(draft)}
        />
      )}

      {routingProvider && (
        <ConfirmDialog
          title="启用本地路由"
          message={`${routingProvider.name} 使用 ${routingProvider.protocol === "openai_chat" ? "Chat Completions" : "Anthropic Messages"}，需要转换为 Responses 协议。`}
          confirmLabel="启用并切换"
          busy={busy === "route"}
          onCancel={() => {
            if (busy !== "route") setRoutingProvider(null);
          }}
          onConfirm={() => void enableRouting()}
        />
      )}

      {deletingProvider && (
        <ConfirmDialog
          title="删除供应商"
          message={`确定删除 ${deletingProvider.name} 吗？保存的 API Key 和模型目录会一并删除。`}
          confirmLabel="删除"
          destructive
          busy={busy === "delete"}
          onCancel={() => {
            if (busy !== "delete") setDeletingProvider(null);
          }}
          onConfirm={() => void removeProvider()}
        />
      )}
    </div>
  );
}

export default App;
