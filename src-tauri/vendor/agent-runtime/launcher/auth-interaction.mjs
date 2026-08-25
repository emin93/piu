function requireObject(value, name) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`invalid ${name}`);
  }
  return value;
}

function requireString(value, name) {
  if (typeof value !== "string" || value.length === 0) throw new Error(`invalid ${name}`);
  return value;
}

function optionalString(value, name) {
  if (value === undefined) return undefined;
  return requireString(value, name);
}

function optionalNumber(value, name) {
  if (value === undefined) return undefined;
  if (typeof value !== "number" || !Number.isFinite(value)) throw new Error(`invalid ${name}`);
  return value;
}

function withOptional(target, key, value) {
  if (value !== undefined) target[key] = value;
  return target;
}

function normalizeLink(link) {
  requireObject(link, "authentication link");
  return withOptional(
    { url: requireString(link.url, "authentication link URL") },
    "label",
    optionalString(link.label, "authentication link label"),
  );
}

function normalizeEvent(event) {
  requireObject(event, "authentication event");
  switch (event.type) {
    case "info": {
      const result = {
        type: "info",
        message: requireString(event.message, "authentication event message"),
      };
      if (event.links !== undefined) {
        if (!Array.isArray(event.links)) throw new Error("invalid authentication event links");
        result.links = event.links.map(normalizeLink);
      }
      return result;
    }
    case "auth_url":
      return withOptional(
        {
          type: "auth_url",
          url: requireString(event.url, "authentication URL"),
        },
        "instructions",
        optionalString(event.instructions, "authentication instructions"),
      );
    case "device_code": {
      const result = {
        type: "device_code",
        userCode: requireString(event.userCode, "device user code"),
        verificationUri: requireString(event.verificationUri, "device verification URL"),
      };
      withOptional(
        result,
        "intervalSeconds",
        optionalNumber(event.intervalSeconds, "device polling interval"),
      );
      return withOptional(
        result,
        "expiresInSeconds",
        optionalNumber(event.expiresInSeconds, "device code expiry"),
      );
    }
    case "progress":
      return {
        type: "progress",
        message: requireString(event.message, "authentication progress message"),
      };
    default:
      throw new Error("unsupported authentication event");
  }
}

function normalizeOption(option) {
  requireObject(option, "authentication option");
  return withOptional(
    {
      id: requireString(option.id, "authentication option id"),
      label: requireString(option.label, "authentication option label"),
    },
    "description",
    optionalString(option.description, "authentication option description"),
  );
}

function normalizePrompt(prompt) {
  requireObject(prompt, "authentication prompt");
  const result = {
    type: prompt.type,
    message: requireString(prompt.message, "authentication prompt message"),
  };
  switch (prompt.type) {
    case "select":
      if (!Array.isArray(prompt.options) || prompt.options.length === 0) {
        throw new Error("invalid authentication options");
      }
      result.options = prompt.options.map(normalizeOption);
      return result;
    case "text":
    case "secret":
    case "manual_code":
      return withOptional(
        result,
        "placeholder",
        optionalString(prompt.placeholder, "authentication prompt placeholder"),
      );
    default:
      throw new Error("unsupported authentication prompt");
  }
}

function cancellationReason(signal) {
  return signal.reason instanceof Error ? signal.reason : new Error("Authentication cancelled");
}

export function createAuthInteraction({ emit }) {
  if (typeof emit !== "function") throw new Error("authentication emitter is required");
  const controller = new AbortController();
  const pending = new Map();
  const retired = new Set();
  let nextPrompt = 0;

  function settlePrompt(id, value) {
    const entry = pending.get(id);
    if (!entry) return false;
    pending.delete(id);
    entry.signal.removeEventListener("abort", entry.abort);
    entry.resolve(value);
    return true;
  }

  function cancel(reason = new Error("Authentication cancelled")) {
    if (!controller.signal.aborted) controller.abort(reason);
  }

  const interaction = {
    signal: controller.signal,
    notify(event) {
      emit({ type: "auth_event", event: normalizeEvent(event) });
    },
    prompt(prompt) {
      controller.signal.throwIfAborted();
      const normalized = normalizePrompt(prompt);
      const id = `auth-${++nextPrompt}`;
      const signal = prompt.signal
        ? AbortSignal.any([controller.signal, prompt.signal])
        : controller.signal;
      return new Promise((resolve, reject) => {
        const abort = () => {
          if (!pending.delete(id)) return;
          retired.add(id);
          emit({ type: "auth_prompt_cancelled", id });
          reject(cancellationReason(signal));
        };
        pending.set(id, { abort, resolve, signal });
        signal.addEventListener("abort", abort, { once: true });
        emit({ type: "auth_prompt", id, prompt: normalized });
      });
    },
  };

  return {
    interaction,
    cancel,
    accept(message) {
      requireObject(message, "authentication command");
      if (message.type === "auth_cancel") {
        cancel();
        return true;
      }
      if (message.type !== "auth_prompt_response") {
        throw new Error("unsupported authentication command");
      }
      const id = requireString(message.id, "authentication prompt id");
      const value = requireString(message.value, "authentication prompt response");
      if (retired.delete(id)) return false;
      if (!settlePrompt(id, value)) throw new Error("unknown authentication prompt id");
      return true;
    },
  };
}

export async function runCodexLogin({ modelRuntime, protocol, emit }) {
  try {
    await modelRuntime.login("openai-codex", "oauth", protocol.interaction);
    emit({ type: "auth_complete" });
    return "complete";
  } catch {
    if (protocol.interaction.signal.aborted) {
      emit({ type: "auth_cancelled" });
      return "cancelled";
    }
    emit({
      type: "auth_failed",
      code: "sign_in_failed",
      message: "Sign-in failed. Try again.",
    });
    return "failed";
  }
}
