#include "mcumgr.h"
#include <iostream>
#include <string>
#include <memory>

struct ConnectionDeleter {
    void operator()(McConnection *p) const { mc_close(&p); }
};
int main() {
    McConnection *raw = nullptr;
    if (mc_open_serial("test", 115200, 60, 128, 512, &raw) != MC_OK) return 1;
    std::unique_ptr<McConnection, ConnectionDeleter> connection(raw);
    McBuffer buffer{nullptr, 0};
    if (mc_image_list_json(connection.get(), &buffer) != MC_OK) return 2;
    std::string json(reinterpret_cast<const char *>(buffer.data), buffer.len);
    if (mc_buffer_free(&buffer) != MC_OK) return 3;
    if (json.find("\"boot_version\":\"test version\"") == std::string::npos) return 4;
    std::cout << json << '\n';
    return 0;
}

