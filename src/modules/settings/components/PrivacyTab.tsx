import {
  createMutation,
  createQuery,
  useQueryClient,
} from "@tanstack/solid-query";
import { CheckCircle, Cloud, Shield, XCircle } from "lucide-solid";
import { settingsApi, settingsKeys } from "~/modules/settings/api";
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  useToast,
} from "~/shared/ui";

export function PrivacyTab() {
  const queryClient = useQueryClient();
  const toast = useToast();

  const settings = createQuery(() => ({
    queryKey: settingsKeys.egress,
    queryFn: () => settingsApi.getEgressSettings(),
  }));

  const grant = createMutation(() => ({
    mutationFn: (provider: string) => settingsApi.grantConsent(provider),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: settingsKeys.egress });
      toast.add({ type: "success", message: "Consent granted" });
    },
  }));

  const revoke = createMutation(() => ({
    mutationFn: (provider: string) => settingsApi.revokeConsent(provider),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: settingsKeys.egress });
      toast.add({ type: "success", message: "Consent revoked" });
    },
  }));

  const providers = [
    {
      id: "kimi",
      name: "Kimi",
      description: "Cloud LLM provider for advanced reasoning",
    },
  ];

  return (
    <div class="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle class="flex items-center gap-2">
            <Shield class="h-5 w-5 text-primary-600" />
            Cloud Provider Consent
          </CardTitle>
          <CardDescription>
            Local providers run on your device with no consent needed. Cloud
            providers require explicit approval.
          </CardDescription>
        </CardHeader>
        <CardContent class="space-y-4">
          {providers.map((provider) => {
            const isConsented = settings.data?.consented_providers.includes(
              provider.id
            );
            return (
              <div class="flex items-center justify-between rounded-lg border border-border p-4">
                <div class="flex items-center gap-3">
                  <Cloud class="h-5 w-5 text-muted-foreground" />
                  <div>
                    <p class="font-medium">{provider.name}</p>
                    <p class="text-sm text-muted-foreground">
                      {provider.description}
                    </p>
                  </div>
                </div>
                <div class="flex items-center gap-3">
                  {isConsented ? (
                    <>
                      <Badge variant="success" class="gap-1">
                        <CheckCircle class="h-3 w-3" />
                        Consented
                      </Badge>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => revoke.mutate(provider.id)}
                        loading={revoke.isPending}
                      >
                        <XCircle class="h-4 w-4" />
                        Revoke
                      </Button>
                    </>
                  ) : (
                    <>
                      <Badge variant="outline">Not consented</Badge>
                      <Button
                        size="sm"
                        onClick={() => grant.mutate(provider.id)}
                        loading={grant.isPending}
                      >
                        Grant
                      </Button>
                    </>
                  )}
                </div>
              </div>
            );
          })}
        </CardContent>
      </Card>
    </div>
  );
}
