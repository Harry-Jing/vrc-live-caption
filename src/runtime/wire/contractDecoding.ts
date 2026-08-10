export type ContractErrorConstructor = new (
  path: string,
  expectation: string,
) => Error;

export function createDecoders(ErrorCtor: ContractErrorConstructor) {
  function record(value: unknown, path: string): Record<string, unknown> {
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
      throw new ErrorCtor(path, "expected an object");
    }

    return value as Record<string, unknown>;
  }

  function exactRecord(
    value: unknown,
    path: string,
    allowedFields: readonly string[],
  ): Record<string, unknown> {
    const decoded = record(value, path);
    const allowed = new Set(allowedFields);
    const unknownField = Object.keys(decoded).find(
      (field) => !allowed.has(field),
    );

    if (unknownField !== undefined) {
      throw new ErrorCtor(`${path}.${unknownField}`, "unknown field");
    }

    return decoded;
  }

  function array(value: unknown, path: string): unknown[] {
    if (!Array.isArray(value)) {
      throw new ErrorCtor(path, "expected an array");
    }

    return value;
  }

  function string(value: unknown, path: string): string {
    if (typeof value !== "string") {
      throw new ErrorCtor(path, "expected a string");
    }

    return value;
  }

  function boolean(value: unknown, path: string): boolean {
    if (typeof value !== "boolean") {
      throw new ErrorCtor(path, "expected a boolean");
    }

    return value;
  }

  function finiteNumber(value: unknown, path: string): number {
    if (typeof value !== "number" || !Number.isFinite(value)) {
      throw new ErrorCtor(path, "expected a finite number");
    }

    return value;
  }

  function safeInteger(
    value: unknown,
    path: string,
    minimum: number,
    maximum?: number,
  ): number {
    if (
      typeof value !== "number" ||
      !Number.isSafeInteger(value) ||
      value < minimum ||
      (maximum !== undefined && value > maximum)
    ) {
      const expectation =
        maximum === undefined
          ? `expected a safe integer greater than or equal to ${String(minimum)}`
          : `expected a safe integer from ${String(minimum)} to ${String(maximum)}`;
      throw new ErrorCtor(path, expectation);
    }

    return value;
  }

  function literal<const Value extends string>(
    value: unknown,
    path: string,
    allowed: readonly Value[],
  ): Value {
    if (typeof value !== "string" || !allowed.includes(value as Value)) {
      throw new ErrorCtor(path, `expected one of ${allowed.join(", ")}`);
    }

    return value as Value;
  }

  return {
    record,
    exactRecord,
    array,
    string,
    boolean,
    finiteNumber,
    safeInteger,
    literal,
  };
}
