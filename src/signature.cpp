#include <openssl/evp.h>
#include <openssl/pem.h>
#include <openssl/x509.h>
#include <spdlog/spdlog.h>
#include <fstream>
#include <sstream>
#include <stdexcept>
#include <vector>

#include "imfwizard/signature.h"

namespace imfwizard
{

static std::string base64_encode(const std::vector<uint8_t>& data)
{
  static const char table[] =
      "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  std::string out;
  out.reserve((data.size() + 2) / 3 * 4);
  for (size_t i = 0; i < data.size(); i += 3)
  {
    uint32_t n = static_cast<uint32_t>(data[i]) << 16;
    if (i + 1 < data.size()) n |= static_cast<uint32_t>(data[i + 1]) << 8;
    if (i + 2 < data.size()) n |= data[i + 2];
    out += table[(n >> 18) & 0x3f];
    out += table[(n >> 12) & 0x3f];
    out += (i + 1 < data.size()) ? table[(n >> 6) & 0x3f] : '=';
    out += (i + 2 < data.size()) ? table[n & 0x3f] : '=';
  }
  return out;
}

static std::vector<uint8_t> compute_sha256(const std::string& data)
{
  std::vector<uint8_t> digest(EVP_MD_size(EVP_sha256()));
  EVP_MD_CTX* ctx = EVP_MD_CTX_new();
  EVP_DigestInit_ex(ctx, EVP_sha256(), nullptr);
  EVP_DigestUpdate(ctx, data.data(), data.size());
  unsigned int len = 0;
  EVP_DigestFinal_ex(ctx, digest.data(), &len);
  EVP_MD_CTX_free(ctx);
  digest.resize(len);
  return digest;
}

static std::vector<uint8_t> sign_data(const std::string& data, EVP_PKEY* pkey)
{
  EVP_MD_CTX* ctx = EVP_MD_CTX_new();
  EVP_DigestSignInit(ctx, nullptr, EVP_sha256(), nullptr, pkey);
  EVP_DigestSignUpdate(ctx, data.data(), data.size());
  size_t sig_len = 0;
  EVP_DigestSignFinal(ctx, nullptr, &sig_len);
  std::vector<uint8_t> sig(sig_len);
  EVP_DigestSignFinal(ctx, sig.data(), &sig_len);
  sig.resize(sig_len);
  EVP_MD_CTX_free(ctx);
  return sig;
}

bool sign_xml(const std::filesystem::path& xml_path, const SignOptions& opts)
{
  namespace fs = std::filesystem;

  if (!fs::exists(xml_path))
  {
    spdlog::error("XML file not found: {}", xml_path.string());
    return false;
  }
  if (!fs::exists(opts.key_file))
  {
    spdlog::error("Key file not found: {}", opts.key_file.string());
    return false;
  }
  if (!fs::exists(opts.cert_file))
  {
    spdlog::error("Certificate file not found: {}", opts.cert_file.string());
    return false;
  }

  // Read XML content
  std::ifstream xml_in(xml_path);
  std::string xml_content((std::istreambuf_iterator<char>(xml_in)),
                          std::istreambuf_iterator<char>());
  xml_in.close();

  // Load private key
  FILE* key_fp = fopen(opts.key_file.c_str(), "r");
  if (!key_fp)
  {
    spdlog::error("Cannot open key file");
    return false;
  }
  EVP_PKEY* pkey = PEM_read_PrivateKey(key_fp, nullptr, nullptr, nullptr);
  fclose(key_fp);
  if (!pkey)
  {
    spdlog::error("Failed to read private key");
    return false;
  }

  // Load certificate for X509Data
  FILE* cert_fp = fopen(opts.cert_file.c_str(), "r");
  X509* cert = nullptr;
  if (cert_fp)
  {
    cert = PEM_read_X509(cert_fp, nullptr, nullptr, nullptr);
    fclose(cert_fp);
  }

  // Compute digest of the document (simplified — real C14N would canonicalize)
  auto digest = compute_sha256(xml_content);
  std::string digest_b64 = base64_encode(digest);

  // Build SignedInfo
  std::ostringstream signed_info;
  signed_info << "<SignedInfo xmlns=\"http://www.w3.org/2000/09/xmldsig#\">";
  signed_info << "<CanonicalizationMethod Algorithm=\"http://www.w3.org/TR/2001/REC-xml-c14n-20010315\"/>";
  signed_info << "<SignatureMethod Algorithm=\"http://www.w3.org/2001/04/xmldsig-more#rsa-sha256\"/>";
  signed_info << "<Reference URI=\"\">";
  signed_info << "<Transforms><Transform Algorithm=\"http://www.w3.org/2000/09/xmldsig#enveloped-signature\"/></Transforms>";
  signed_info << "<DigestMethod Algorithm=\"http://www.w3.org/2001/04/xmlenc#sha256\"/>";
  signed_info << "<DigestValue>" << digest_b64 << "</DigestValue>";
  signed_info << "</Reference></SignedInfo>";

  // Sign the SignedInfo
  auto sig = sign_data(signed_info.str(), pkey);
  std::string sig_b64 = base64_encode(sig);
  EVP_PKEY_free(pkey);

  // Build Signature element
  std::ostringstream sig_elem;
  sig_elem << "<Signature xmlns=\"http://www.w3.org/2000/09/xmldsig#\">";
  sig_elem << signed_info.str();
  sig_elem << "<SignatureValue>" << sig_b64 << "</SignatureValue>";
  if (cert)
  {
    // Encode cert as DER then base64
    int der_len = i2d_X509(cert, nullptr);
    std::vector<uint8_t> der(der_len);
    uint8_t* p = der.data();
    i2d_X509(cert, &p);
    sig_elem << "<KeyInfo><X509Data><X509Certificate>"
             << base64_encode(der) << "</X509Certificate></X509Data></KeyInfo>";
    X509_free(cert);
  }
  sig_elem << "</Signature>";

  // Insert before closing root element
  auto close_pos = xml_content.rfind("</");
  if (close_pos == std::string::npos)
  {
    spdlog::error("Cannot find closing root element");
    return false;
  }
  xml_content.insert(close_pos, sig_elem.str());

  // Write signed XML
  std::ofstream xml_out(xml_path);
  xml_out << xml_content;
  xml_out.close();

  spdlog::info("XML signed: {}", xml_path.string());
  return true;
}

} // namespace imfwizard
