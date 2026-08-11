const timeFormatter = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});

export function formatTime(timestampMs: number) {
  return timeFormatter.format(new Date(timestampMs));
}
