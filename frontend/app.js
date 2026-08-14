const invoke = window.__TAURI__.core.invoke;
const loginButton = document.querySelector("#login-button");
const configButton = document.querySelector("#config-button");
const dialog = document.querySelector("#config-dialog");
const form = document.querySelector("#config-form");
const apiUrl = document.querySelector("#api-url");
const apiKey = document.querySelector("#api-key");
const closeDialog = document.querySelector("#close-dialog");
const revealKey = document.querySelector("#reveal-key");
const saveButton = document.querySelector("#save-button");
const toast = document.querySelector("#toast");
let toastTimer;

function showToast(message, isError = false) {
  clearTimeout(toastTimer);
  toast.textContent = message;
  toast.classList.toggle("error", isError);
  toast.classList.add("visible");
  toastTimer = setTimeout(() => toast.classList.remove("visible"), 2800);
}

function errorMessage(error) {
  return typeof error === "string" ? error : error?.message || String(error);
}

loginButton.addEventListener("click", async () => {
  loginButton.disabled = true;
  try {
    await invoke("start_official_login");
    showToast("已启动 Codex 官方登录");
  } catch (error) {
    showToast(errorMessage(error), true);
  } finally {
    loginButton.disabled = false;
  }
});

configButton.addEventListener("click", async () => {
  apiUrl.value = "";
  apiKey.value = "";
  apiKey.placeholder = "输入 API Key";
  try {
    const config = await invoke("load_proxy_config");
    apiUrl.value = config.apiUrl;
    if (config.hasApiKey) apiKey.placeholder = "已配置，输入新值后替换";
  } catch (error) {
    showToast(errorMessage(error), true);
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
form.addEventListener("submit", async (event) => {
  event.preventDefault();
  saveButton.disabled = true;
  saveButton.textContent = "保存中";
  try {
    await invoke("save_proxy_config", { apiUrl: apiUrl.value, apiKey: apiKey.value });
    apiKey.value = "";
    apiKey.type = "password";
    dialog.close();
    showToast("配置已保存，重启 Codex 后生效");
  } catch (error) {
    showToast(errorMessage(error), true);
  } finally {
    saveButton.disabled = false;
    saveButton.textContent = "保存";
  }
});
