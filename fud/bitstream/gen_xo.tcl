if { $::argc != 1 } {
    puts "ERROR: Program \"$::argv0\" requires 1 argument!\n"
    puts "Usage: $::argv0 <xoname>\n"
    exit
}

set xoname [lindex $::argv 0]
set path_to_packaged "./packaged_kernel"

# Make a temporary Vivado project.
create_project -force kernel_pack "./tmp_kernel_pack"

# Add all Verilog files in the current working directory.
add_files -norecurse [glob *.v *.sv]

# I don't really understand any of this.
ipx::package_project -root_dir $path_to_packaged -vendor capra.cs.cornell.edu -library RTLKernel -taxonomy /KernelIP -import_files -set_current false
ipx::unload_core $path_to_packaged/component.xml
ipx::edit_ip_in_project -upgrade true -name tmp_edit_project -directory $path_to_packaged $path_to_packaged/component.xml
set_property sdx_kernel true [ipx::current_core]
set_property sdx_kernel_type rtl [ipx::current_core]

# Declare bus interfaces.
ipx::associate_bus_interfaces -busif s_axi_control -clock ap_clk [ipx::current_core]
ipx::associate_bus_interfaces -busif m0_axi -clock ap_clk [ipx::current_core]

# Close & save the temporary project.
ipx::update_checksums [ipx::current_core]
ipx::save_core [ipx::current_core]
close_project -delete

# Package the project as an .xo file.
package_xo -xo_path ${xoname} -kernel_name Toplevel -ip_directory ${path_to_packaged} -kernel_xml ./kernel.xml
