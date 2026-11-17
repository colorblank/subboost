
type LocalAdminCredentialInput = {
  username: string;
  password: string;
  passwordConfirm: string;
};

export function getLocalAdminSetupCredentialError(input: LocalAdminCredentialInput, minimumPasswordCharacters: number): string {
  const username = input.username.trim();
  const password = input.password.trim();
  const passwordConfirm = input.passwordConfirm.trim();

  if (!username) return "请输入管理员账号";
  let characterCount = 0;
  for (const _character of password) characterCount += 1;
  if (characterCount < minimumPasswordCharacters) return `密码至少需要 ${minimumPasswordCharacters} 个字符`;
  if (password !== passwordConfirm) return "两次输入的密码不一致，请重新确认";
  return "";
}
