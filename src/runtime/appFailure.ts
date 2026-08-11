export type AppFailure<Code extends string | null = string | null> = Readonly<{
  code: Code;
  message: string;
}>;

function readStringField(value: unknown, field: string) {
  if (
    typeof value !== "object" ||
    value === null ||
    !(field in value) ||
    typeof value[field as keyof typeof value] !== "string"
  ) {
    return null;
  }

  const fieldValue = value[field as keyof typeof value] as string;
  return fieldValue.length > 0 ? fieldValue : null;
}

export function normalizeAppFailure(
  cause: unknown,
  fallbackMessage: string,
): AppFailure {
  const message =
    (typeof cause === "string" && cause.length > 0 ? cause : null) ??
    readStringField(cause, "message") ??
    fallbackMessage;

  return {
    code: readStringField(cause, "code"),
    message,
  };
}
