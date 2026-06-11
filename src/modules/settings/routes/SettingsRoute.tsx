import { ClipboardList, Palette, Shield } from "lucide-solid";
import { AppearanceTab } from "~/modules/settings/components/AppearanceTab";
import { AuditTab } from "~/modules/settings/components/AuditTab";
import { PrivacyTab } from "~/modules/settings/components/PrivacyTab";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "~/shared/ui";

export default function SettingsRoute() {
  return (
    <div class="mx-auto max-w-3xl p-4">
      <div class="mb-6">
        <h1 class="text-2xl font-bold">Settings</h1>
        <p class="text-sm text-muted-foreground">
          Manage your preferences and privacy
        </p>
      </div>

      <Tabs defaultValue="privacy">
        <TabsList class="mb-4">
          <TabsTrigger value="privacy">
            <Shield class="mr-2 h-4 w-4" />
            Privacy
          </TabsTrigger>
          <TabsTrigger value="audit">
            <ClipboardList class="mr-2 h-4 w-4" />
            Audit Log
          </TabsTrigger>
          <TabsTrigger value="appearance">
            <Palette class="mr-2 h-4 w-4" />
            Appearance
          </TabsTrigger>
        </TabsList>

        <TabsContent value="privacy">
          <PrivacyTab />
        </TabsContent>
        <TabsContent value="audit">
          <AuditTab />
        </TabsContent>
        <TabsContent value="appearance">
          <AppearanceTab />
        </TabsContent>
      </Tabs>
    </div>
  );
}
