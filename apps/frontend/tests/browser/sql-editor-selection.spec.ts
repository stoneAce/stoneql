import { test, expect } from '@playwright/test';

// The cursor's line is highlighted, and CodeMirror draws the selection behind the text: an opaque
// line highlight hid the selection on that line, so selecting a one-line query showed nothing.
for (const theme of ['light', 'dark'])
  test(`a selection on the cursor's line stays visible in the SQL editor (${theme})`, async ({ page }) => {
    await page.addInitScript((value) => localStorage.setItem('stoneql-theme', value), theme);
    await page.goto('/');
    await page.getByRole('tab', { name: 'SQL', exact: true }).click();
    await page.locator('.cm-content').fill('SELECT id, name FROM users WHERE id = 1');
    const activeLine = page.locator('.cm-activeLine');
    await expect(activeLine).not.toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
    await page.keyboard.press('ControlOrMeta+a');
    await expect(page.locator('.cm-selectionBackground')).toHaveCount(1);
    await expect(activeLine).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
    await page.keyboard.press('End');
    await expect(activeLine).not.toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  });
