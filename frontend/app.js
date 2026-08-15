const invoke = window.__TAURI__.core.invoke;
const loginButton = document.querySelector("#login-button");
const loginLabel = document.querySelector("#login-label");
const configButton = document.querySelector("#config-button");
const dialog = document.querySelector("#config-dialog");
const form = document.querySelector("#config-form");
const apiUrl = document.querySelector("#api-url");
const apiKey = document.querySelector("#api-key");
const closeDialog = document.querySelector("#close-dialog");
const revealKey = document.querySelector("#reveal-key");
const saveButton = document.querySelector("#save-button");
const toast = document.querySelector("#toast");
const errorDialog = document.querySelector("#error-dialog");
const errorMessageBox = document.querySelector("#error-message");
const closeErrorDialog = document.querySelector("#close-error-dialog");
const dismissError = document.querySelector("#dismiss-error");
let toastTimer;
let loginInProgress = false;
let cancelInProgress = false;
let dialogToResume = null;

function showToast(message) {
  clearTimeout(toastTimer);
  toast.textContent = message;
  toast.classList.add("visible");
  toastTimer = setTimeout(() => toast.classList.remove("visible"), 2800);
}

function showError(message, resumeDialog = null) {
  dialogToResume = resumeDialog?.open ? resumeDialog : null;
  if (dialogToResume) dialogToResume.close();
  errorMessageBox.textContent = message;
  if (!errorDialog.open) errorDialog.showModal();
}

function dismissErrorDialog() {
  errorDialog.close();
}

function errorMessage(error) {
  return typeof error === "string" ? error : error?.message || String(error);
}

loginButton.addEventListener("click", async () => {
  if (loginInProgress) {
    if (cancelInProgress) return;
    cancelInProgress = true;
    loginButton.disabled = true;
    loginLabel.textContent = "正在取消";
    try {
      await invoke("cancel_official_login");
    } catch (error) {
      cancelInProgress = false;
      loginButton.disabled = false;
      loginLabel.textContent = "取消登录";
      showError(errorMessage(error));
    }
    return;
  }

  loginInProgress = true;
  loginLabel.textContent = "取消登录";
  configButton.disabled = true;
  try {
    const result = await invoke("start_official_login");
    showToast(`已切换官方登录，已统一 ${result.rolloutFilesUpdated} 个会话文件`);
  } catch (error) {
    const message = errorMessage(error);
    if (message === "官方登录已取消") showToast(message);
    else showError(message);
  } finally {
    loginInProgress = false;
    cancelInProgress = false;
    loginButton.disabled = false;
    configButton.disabled = false;
    loginLabel.textContent = "官方登录";
  }
});

configButton.addEventListener("click", async () => {
  apiUrl.value = "";
  apiKey.value = "";
  apiKey.placeholder = "输入 API Key";
  try {
    const config = await invoke("load_proxy_config");
    apiUrl.value = config.apiUrl;
    apiKey.required = !config.hasApiKey;
    if (config.hasApiKey) apiKey.placeholder = "已配置，留空继续使用";
  } catch (error) {
    apiKey.required = true;
    showError(errorMessage(error));
    return;
  }
  dialog.showModal();
  (apiUrl.value ? apiKey : apiUrl).focus();
});

closeDialog.addEventListener("click", () => dialog.close());
dialog.addEventListener("click", (event) => {
  if (event.target === dialog) dialog.close();
});
revealKey.addEventListener("click", () => {
  const reveal = apiKey.type === "password";
  apiKey.type = reveal ? "text" : "password";
  revealKey.setAttribute("aria-label", reveal ? "隐藏 API Key" : "显示 API Key");
});
closeErrorDialog.addEventListener("click", dismissErrorDialog);
dismissError.addEventListener("click", dismissErrorDialog);
errorDialog.addEventListener("click", (event) => {
  if (event.target === errorDialog) dismissErrorDialog();
});
errorDialog.addEventListener("close", () => {
  if (!dialogToResume) return;
  const resume = dialogToResume;
  dialogToResume = null;
  resume.showModal();
});
form.addEventListener("submit", async (event) => {
  event.preventDefault();
  saveButton.disabled = true;
  loginButton.disabled = true;
  configButton.disabled = true;
  saveButton.textContent = "保存中";
  try {
    const result = await invoke("save_proxy_config", { apiUrl: apiUrl.value, apiKey: apiKey.value });
    apiKey.value = "";
    apiKey.type = "password";
    dialog.close();
    showToast(`配置已保存，已统一 ${result.rolloutFilesUpdated} 个会话文件`);
  } catch (error) {
    showError(errorMessage(error), dialog);
  } finally {
    saveButton.disabled = false;
    loginButton.disabled = false;
    configButton.disabled = false;
    saveButton.textContent = "保存";
  }
});
