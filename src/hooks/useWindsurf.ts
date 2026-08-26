import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { windsurfApi } from "@/lib/api/windsurf";

export const windsurfKeys = {
  all: ["windsurf"] as const,
  accounts: () => [...windsurfKeys.all, "accounts"] as const,
  status: () => [...windsurfKeys.all, "status"] as const,
};

export function useWindsurfAccounts() {
  return useQuery({
    queryKey: windsurfKeys.accounts(),
    queryFn: () => windsurfApi.listAccounts(),
  });
}

export function useWindsurfStatus(enabled = true) {
  return useQuery({
    queryKey: windsurfKeys.status(),
    queryFn: () => windsurfApi.getStatus(),
    enabled,
  });
}

export function useWindsurfActions() {
  const queryClient = useQueryClient();
  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: windsurfKeys.all });
    void queryClient.invalidateQueries({ queryKey: ["providers"] });
  };

  const importLocal = useMutation({
    mutationFn: () => windsurfApi.importFromLocal(),
    onSuccess: invalidate,
  });
  const addByToken = useMutation({
    mutationFn: ({ token, label }: { token: string; label?: string }) =>
      windsurfApi.addByToken(token, label),
    onSuccess: invalidate,
  });
  const addByPassword = useMutation({
    mutationFn: ({
      email,
      password,
      label,
    }: {
      email: string;
      password: string;
      label?: string;
    }) => windsurfApi.addByPassword(email, password, label),
    onSuccess: invalidate,
  });
  const deleteAccount = useMutation({
    mutationFn: (accountId: string) => windsurfApi.deleteAccount(accountId),
    onSuccess: invalidate,
  });
  const switchAccount = useMutation({
    mutationFn: (accountId: string) => windsurfApi.switchAccount(accountId),
    onSuccess: invalidate,
  });
  const detectAppPath = useMutation({
    mutationFn: () => windsurfApi.detectAppPath(true),
    onSuccess: invalidate,
  });

  return {
    importLocal,
    addByToken,
    addByPassword,
    deleteAccount,
    switchAccount,
    detectAppPath,
  };
}
