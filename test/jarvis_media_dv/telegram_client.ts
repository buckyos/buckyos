export type TelegramMediaRef = {
  messageId: number;
  mimeType: string;
  fileName?: string;
};

export type TelegramObservedMessage = {
  messageId: number;
  text: string;
  media?: TelegramMediaRef;
};

export type TelegramClientOptions = {
  apiId: number;
  apiHash: string;
  phoneNumber?: string;
  phoneCode?: string;
  password?: string;
  session?: string;
  sessionFile: string;
  botUsername: string;
  connectionRetries: number;
  promptValue: (label: string, secret?: boolean) => Promise<string>;
};

type GramMessage = {
  id?: number;
  out?: boolean;
  message?: string;
  text?: string;
  media?: { className?: string; document?: GramDocument };
  document?: GramDocument;
  photo?: unknown;
};

type GramDocument = {
  mimeType?: string;
  attributes?: Array<{ className?: string; fileName?: string }>;
};

type GramClient = {
  start: (options: Record<string, unknown>) => Promise<void>;
  getEntity: (entity: string) => Promise<unknown>;
  getMessages: (entity: unknown, options: Record<string, unknown>) => Promise<GramMessage[]>;
  sendMessage: (entity: unknown, options: Record<string, unknown>) => Promise<GramMessage>;
  sendFile: (entity: unknown, options: Record<string, unknown>) => Promise<GramMessage>;
  disconnect: () => Promise<void>;
  session: { save: () => string };
};

async function readSession(path: string): Promise<string> {
  try {
    return (await Deno.readTextFile(path)).trim();
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return "";
    throw error;
  }
}

async function writeSession(path: string, session: string): Promise<void> {
  const parent = path.replace(/[\\/][^\\/]+$/, "");
  if (parent && parent !== path) await Deno.mkdir(parent, { recursive: true });
  await Deno.writeTextFile(path, `${session.trim()}\n`, { mode: 0o600 });
}

function normalizeBotUsername(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) throw new Error("Telegram bot username is empty");
  return trimmed.startsWith("@") ? trimmed : `@${trimmed}`;
}

function messageId(message: GramMessage): number {
  const value = Number(message.id);
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error("Telegram returned a message without a valid id");
  }
  return value;
}

function documentFileName(document: GramDocument | undefined): string | undefined {
  return document?.attributes?.find((attribute) =>
    attribute.className === "DocumentAttributeFilename" && attribute.fileName
  )?.fileName;
}

function mediaRef(message: GramMessage): TelegramMediaRef | undefined {
  const media = message.media;
  if (!media) return undefined;
  if (message.photo || media.className === "MessageMediaPhoto") {
    return { messageId: messageId(message), mimeType: "image/jpeg" };
  }
  const document = message.document ?? media.document;
  if (document) {
    return {
      messageId: messageId(message),
      mimeType: document.mimeType?.trim() || "application/octet-stream",
      fileName: documentFileName(document),
    };
  }
  return undefined;
}

export class TelegramDvClient {
  private readonly options: TelegramClientOptions;
  private client?: GramClient;
  private bot?: unknown;

  constructor(options: TelegramClientOptions) {
    this.options = options;
  }

  async connect(): Promise<void> {
    const [{ TelegramClient }, { StringSession }] = await Promise.all([
      import("telegram"),
      import("telegram/sessions/index.js"),
    ]);
    const savedSession = this.options.session?.trim() ||
      await readSession(this.options.sessionFile);
    const client = new TelegramClient(
      new StringSession(savedSession),
      this.options.apiId,
      this.options.apiHash,
      { connectionRetries: this.options.connectionRetries },
    ) as unknown as GramClient;
    let lastAuthError = "";
    let configuredPhoneCode = this.options.phoneCode?.trim();
    await client.start({
      phoneNumber: async () =>
        this.options.phoneNumber?.trim() ||
        await this.options.promptValue("Telegram phone number"),
      phoneCode: async () => {
        const code = configuredPhoneCode;
        configuredPhoneCode = undefined;
        return code || await this.options.promptValue("Telegram login code", true);
      },
      password: async () =>
        this.options.password?.trim() ||
        await this.options.promptValue("Telegram 2FA password", true),
      onError: (error: unknown) => {
        lastAuthError = String(error);
        console.error(`[telegram-auth] ${lastAuthError}`);
      },
    });
    const session = client.session.save();
    if (!session) {
      throw new Error(`Telegram login did not produce a session${lastAuthError ? `: ${lastAuthError}` : ""}`);
    }
    await writeSession(this.options.sessionFile, session);
    this.bot = await client.getEntity(normalizeBotUsername(this.options.botUsername));
    this.client = client;
  }

  async disconnect(): Promise<void> {
    if (this.client) await this.client.disconnect();
    this.client = undefined;
    this.bot = undefined;
  }

  async latestMessageId(): Promise<number> {
    const messages = await this.requireClient().getMessages(this.requireBot(), { limit: 1 });
    return messages.length > 0 ? messageId(messages[0]) : 0;
  }

  async send(input: {
    text: string;
    file?: string;
    replyTo?: number;
  }): Promise<number> {
    const client = this.requireClient();
    const bot = this.requireBot();
    const replyTo = input.replyTo ? { replyTo: input.replyTo } : {};
    const sent = input.file
      ? await client.sendFile(bot, {
        file: input.file,
        caption: input.text,
        ...replyTo,
      })
      : await client.sendMessage(bot, {
        message: input.text,
        ...replyTo,
      });
    return messageId(sent);
  }

  async messagesAfter(afterId: number): Promise<TelegramObservedMessage[]> {
    const messages = await this.requireClient().getMessages(this.requireBot(), {
      limit: 100,
      minId: afterId,
      reverse: true,
    });
    return messages
      .filter((message) => !message.out && messageId(message) > afterId)
      .sort((left, right) => messageId(left) - messageId(right))
      .map((message) => ({
        messageId: messageId(message),
        text: (message.message ?? message.text ?? "").trim(),
        media: mediaRef(message),
      }));
  }

  private requireClient(): GramClient {
    if (!this.client) throw new Error("Telegram client is not connected");
    return this.client;
  }

  private requireBot(): unknown {
    if (!this.bot) throw new Error("Telegram bot is not resolved");
    return this.bot;
  }
}
