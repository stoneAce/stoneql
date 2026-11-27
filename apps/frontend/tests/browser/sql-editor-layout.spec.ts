import { test, expect, type Page } from '@playwright/test';

const editorHeight = async (page: Page) => (await page.locator('.sql-editor').boundingBox())!.height;

test('the tab rows above the SQL editor never show a vertical scrollbar', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('tab', { name: 'SQL', exact: true }).click();
  for (const row of ['.connection-tabs', '.view-tabs', '.query-tabs'])
    await expect(page.locator(row), row).toHaveCSS('overflow-y', 'hidden');
});

test('the SQL editor resizes by dragging its handle and with the arrow keys', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('tab', { name: 'SQL', exact: true }).click();
  const handle = page.getByRole('separator', { name: 'Resize SQL editor' });
  const before = await editorHeight(page);
  const box = (await handle.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 100, { steps: 5 });
  await page.mouse.up();
  expect(await editorHeight(page)).toBeCloseTo(before + 100, -1);

  await handle.focus();
  const dragged = await editorHeight(page);
  await page.keyboard.press('ArrowUp');
  expect(await editorHeight(page)).toBeCloseTo(dragged - 24, 0);
  for (let i = 0; i < 20; i++) await page.keyboard.press('ArrowUp');
  expect(await editorHeight(page)).toBeCloseTo(96, 0);
});
