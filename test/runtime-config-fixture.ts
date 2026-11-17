export function useRuntimeConfig() {
  return {
    config: {
      autoUpdateIntervalPolicy: { defaultHours: 12, minHours: 0.1, stepHours: 0.1, requireIntegerHours: false },
      minimumPasswordCharacters: 10,
    },
    error: "",
  };
}
