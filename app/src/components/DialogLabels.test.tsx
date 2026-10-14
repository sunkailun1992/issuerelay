import { screen } from "@testing-library/react";
import type { Lang } from "@/i18n";
import ja from "@/i18n/locales/ja.json";
import ko from "@/i18n/locales/ko.json";
import en from "@/i18n/locales/en.json";
import zhCN from "@/i18n/locales/zh-CN.json";
import zhTW from "@/i18n/locales/zh-TW.json";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogTitle } from "@/components/ui/dialog";
import { renderWithProviders } from "@/test/render";

// components/ui/dialog.tsx comes from the shadcn CLI, which writes the label of its two close
// buttons in English. The panel's languages need them translated, so the file has been changed in
// those two places (`common.close`); this test fails if a regenerated file brings the English back.
const CLOSE: [Lang, string][] = [
  ["en", en.common.close],
  ["zh-CN", zhCN.common.close],
  ["zh-TW", zhTW.common.close],
  ["ja", ja.common.close],
  ["ko", ko.common.close],
];

test.each(CLOSE)("the close button of a dialog is named in %s", async (lang, label) => {
  await renderWithProviders(
    <Dialog open>
      <DialogContent>
        <DialogTitle>Title</DialogTitle>
        <DialogDescription>Text</DialogDescription>
      </DialogContent>
    </Dialog>,
    { lang },
  );
  const close = await screen.findByRole("button", { name: label });
  expect(close).toBeInTheDocument();
  // A language other than English does not leave the English name behind.
  if (lang !== "en") expect(screen.queryByRole("button", { name: "Close" })).not.toBeInTheDocument();
});

test.each(CLOSE)("the close button of a dialog's footer is named in %s", async (lang, label) => {
  await renderWithProviders(
    <Dialog open>
      <DialogContent showCloseButton={false}>
        <DialogTitle>Title</DialogTitle>
        <DialogDescription>Text</DialogDescription>
        <DialogFooter showCloseButton />
      </DialogContent>
    </Dialog>,
    { lang },
  );
  expect(await screen.findByRole("button", { name: label })).toBeInTheDocument();
  if (lang !== "en") expect(screen.queryByRole("button", { name: "Close" })).not.toBeInTheDocument();
});
